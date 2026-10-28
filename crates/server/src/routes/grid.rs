use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};

use crate::routes::{fail_code, session_id, Failure};
use crate::settings::SettingsStore;

pub const GRID_MAX_TILES: usize = 9;
pub const GRID_LAYOUTS: [&str; 4] = ["auto", "1", "2", "3"];
pub const GRID_QUALITIES: [&str; 2] = ["main", "sub"];
const DEVICE_LIMIT: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridTile {
    pub device: String,
    pub channel: u32,
    pub quality: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridSpec {
    pub layout: String,
    pub tiles: Vec<GridTile>,
}

impl Default for GridSpec {
    fn default() -> Self {
        Self {
            layout: GRID_LAYOUTS[0].to_string(),
            tiles: Vec::new(),
        }
    }
}

fn validate(spec: &GridSpec) -> Result<(), String> {
    if !GRID_LAYOUTS.contains(&spec.layout.as_str()) {
        return Err(format!("layout must be one of {}", GRID_LAYOUTS.join(", ")));
    }
    if spec.tiles.len() > GRID_MAX_TILES {
        return Err(format!(
            "a grid holds at most {GRID_MAX_TILES} tiles, not {}",
            spec.tiles.len()
        ));
    }
    let mut seen: Vec<(&str, u32)> = Vec::with_capacity(spec.tiles.len());
    for tile in &spec.tiles {
        let device = tile.device.trim();
        if device.is_empty() {
            return Err("every tile names a device".to_string());
        }
        if device.len() > DEVICE_LIMIT {
            return Err(format!("a device id is at most {DEVICE_LIMIT} characters"));
        }
        if tile.channel < 1 {
            return Err("a tile channel starts at 1".to_string());
        }
        if !GRID_QUALITIES.contains(&tile.quality.as_str()) {
            return Err(format!(
                "a tile stream is one of {}",
                GRID_QUALITIES.join(", ")
            ));
        }
        if seen.contains(&(device, tile.channel)) {
            return Err(format!("{device} channel {} appears twice", tile.channel));
        }
        seen.push((device, tile.channel));
    }
    Ok(())
}

impl GridSpec {
    pub fn normalised(mut self) -> Result<Self, String> {
        for tile in self.tiles.iter_mut() {
            tile.device = tile.device.trim().to_string();
        }
        validate(&self)?;
        Ok(self)
    }
}

#[get("/grid")]
fn read_grid(
    store: &State<SettingsStore>,
    cookies: &CookieJar<'_>,
) -> Result<Json<GridSpec>, Failure> {
    session_id(cookies)?;
    Ok(Json(store.grid_spec()))
}

#[post("/grid", data = "<body>")]
fn write_grid(
    store: &State<SettingsStore>,
    cookies: &CookieJar<'_>,
    body: Json<GridSpec>,
) -> Result<Json<GridSpec>, Failure> {
    session_id(cookies)?;
    let spec = body
        .into_inner()
        .normalised()
        .map_err(|message| fail_code(Status::BadRequest, "grid_invalid", message))?;
    store
        .set_grid_spec(&spec)
        .map_err(|error| fail_code(Status::InternalServerError, "grid_write_failed", error))?;
    Ok(Json(spec))
}

pub fn routes() -> Vec<rocket::route::Route> {
    routes![read_grid, write_grid]
}

#[cfg(test)]
mod tests {
    use super::*;
    use proview_core::config::{Config, ProviderKind};
    use proview_core::provider::Mode;
    use rocket::http::ContentType;
    use rocket::local::blocking::Client;
    use std::path::{Path, PathBuf};

    static SLOT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let slot = SLOT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("zha4014-grid-{}-{slot}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn fixture_dir(tag: &str) -> PathBuf {
        let dir = temp_dir(tag);
        std::fs::write(
            dir.join("account.json"),
            r#"{"email":"fixture@example.invalid","password":"fixture-password"}"#,
        )
        .expect("account fixture");
        std::fs::write(
            dir.join("devices.json"),
            r#"[{"id":"gate","name":"Gate camera","model":"XM-IPC","online":true,"channels":1,"lan_host":null}]"#,
        )
        .expect("device fixture");
        dir
    }

    fn fixture_config(dir: &Path) -> Config {
        Config {
            provider: ProviderKind::Fixture,
            cloud_base: String::new(),
            share_base: proview_core::cloud::attributes::SHARE_BASE.to_string(),
            device_user: "admin".to_string(),
            device_pass: String::new(),
            fixture_dir: dir.to_string_lossy().to_string(),
            device_onvif: None,
            device_host: None,
            device_rtsp: None,
            session_ttl_secs: 60,
        }
    }

    fn client(tag: &str, signed_in: bool) -> (Client, PathBuf, PathBuf) {
        let data = temp_dir(&format!("{tag}-data"));
        let fixtures = fixture_dir(&format!("{tag}-fixtures"));
        let settings = crate::settings::SettingsStore::load(Some(&data));
        let figment = rocket::Config::figment()
            .merge(("secret_key", "aGVsbG8gd29ybGQgdGhpcyBpcyBhIHRlc3Qga2V5ISE="));
        let rocket = crate::app::build_with_state(
            figment,
            fixture_config(&fixtures),
            crate::config::ModeStore::new(Mode::default(), None),
            settings,
        );
        let client = Client::tracked(rocket).expect("rocket");
        if signed_in {
            let status = client
                .post("/api/login")
                .json(&serde_json::json!({
                    "email": "fixture@example.invalid",
                    "password": "fixture-password"
                }))
                .dispatch()
                .status();
            assert_eq!(status, Status::Ok);
        }
        (client, data, fixtures)
    }

    fn tile(device: &str, channel: u32, quality: &str) -> GridTile {
        GridTile {
            device: device.to_string(),
            channel,
            quality: quality.to_string(),
        }
    }

    fn post(client: &Client, spec: serde_json::Value) -> (Status, serde_json::Value) {
        let response = client
            .post("/api/grid")
            .header(ContentType::JSON)
            .body(spec.to_string())
            .dispatch();
        let status = response.status();
        let body = response.into_string().unwrap_or_default();
        (
            status,
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null),
        )
    }

    fn cleanup(data: PathBuf, fixtures: PathBuf) {
        let _ = std::fs::remove_dir_all(data);
        let _ = std::fs::remove_dir_all(fixtures);
    }

    #[test]
    fn an_empty_grid_is_the_default() {
        let (client, data, fixtures) = client("default", true);
        let response = client.get("/api/grid").dispatch();
        assert_eq!(response.status(), Status::Ok);
        let spec: GridSpec = serde_json::from_str(&response.into_string().unwrap()).expect("spec");
        assert_eq!(spec.layout, "auto");
        assert!(spec.tiles.is_empty());
        cleanup(data, fixtures);
    }

    #[test]
    fn the_route_requires_a_session() {
        let (client, data, fixtures) = client("anon", false);
        assert_eq!(
            client.get("/api/grid").dispatch().status(),
            Status::Unauthorized
        );
        let (status, _) = post(
            &client,
            serde_json::json!({ "layout": "auto", "tiles": [] }),
        );
        assert_eq!(status, Status::Unauthorized);
        cleanup(data, fixtures);
    }

    #[test]
    fn a_saved_grid_survives_the_process_and_lands_in_the_settings_file() {
        let (client, data, fixtures) = client("roundtrip", true);
        let body = serde_json::json!({
            "layout": "2",
            "tiles": [
                { "device": "gate", "channel": 1, "quality": "main" },
                { "device": "porch", "channel": 1, "quality": "sub" }
            ]
        });
        let (status, answer) = post(&client, body.clone());
        assert_eq!(status, Status::Ok, "{answer}");
        let read: GridSpec = serde_json::from_str(
            &client
                .get("/api/grid")
                .dispatch()
                .into_string()
                .expect("body"),
        )
        .expect("spec");
        assert_eq!(read.layout, "2");
        assert_eq!(read.tiles.len(), 2);
        assert_eq!(read.tiles[1].device, "porch");
        assert_eq!(read.tiles[1].quality, "sub");

        let raw = std::fs::read_to_string(data.join("settings.json")).expect("settings file");
        let file: serde_json::Value = serde_json::from_str(&raw).expect("json");
        assert_eq!(file["grid"]["layout"], "2");
        assert_eq!(file["grid"]["tiles"][0]["device"], "gate");

        let reopened = crate::settings::SettingsStore::load(Some(&data));
        assert_eq!(reopened.grid_spec().layout, "2");
        assert_eq!(reopened.grid_spec().tiles.len(), 2);
        cleanup(data, fixtures);
    }

    #[test]
    fn a_settings_save_keeps_the_grid_it_did_not_touch() {
        let (client, data, fixtures) = client("keep", true);
        let (status, _) = post(
            &client,
            serde_json::json!({ "layout": "3", "tiles": [{ "device": "gate", "channel": 1, "quality": "sub" }] }),
        );
        assert_eq!(status, Status::Ok);
        let response = client
            .post("/api/settings")
            .header(ContentType::JSON)
            .body(
                serde_json::json!({ "values": { "PROVIEW_DEVICE_USER": "operator" } }).to_string(),
            )
            .dispatch();
        assert_eq!(response.status(), Status::Ok);
        let reopened = crate::settings::SettingsStore::load(Some(&data));
        assert_eq!(reopened.grid_spec().layout, "3");
        assert_eq!(reopened.grid_spec().tiles.len(), 1);
        std::env::remove_var("PROVIEW_DEVICE_USER");
        cleanup(data, fixtures);
    }

    #[test]
    fn an_unknown_layout_is_refused_with_a_code() {
        let (client, data, fixtures) = client("layout", true);
        let (status, answer) = post(&client, serde_json::json!({ "layout": "9", "tiles": [] }));
        assert_eq!(status, Status::BadRequest);
        assert_eq!(answer["code"], "grid_invalid");
        cleanup(data, fixtures);
    }

    #[test]
    fn a_repeated_device_channel_and_an_overfull_grid_are_refused() {
        let (client, data, fixtures) = client("tiles", true);
        let twice = serde_json::json!({
            "layout": "auto",
            "tiles": [
                { "device": "gate", "channel": 1, "quality": "main" },
                { "device": "gate", "channel": 1, "quality": "sub" }
            ]
        });
        let (status, answer) = post(&client, twice);
        assert_eq!(status, Status::BadRequest);
        assert!(answer["message"]
            .as_str()
            .unwrap_or_default()
            .contains("twice"));

        let many: Vec<serde_json::Value> = (1..=GRID_MAX_TILES + 1)
            .map(|index| {
                serde_json::json!({ "device": format!("cam-{index}"), "channel": 1, "quality": "main" })
            })
            .collect();
        let (status, answer) = post(
            &client,
            serde_json::json!({ "layout": "auto", "tiles": many }),
        );
        assert_eq!(status, Status::BadRequest);
        assert!(answer["message"]
            .as_str()
            .unwrap_or_default()
            .contains("at most"));
        cleanup(data, fixtures);
    }

    #[test]
    fn a_tile_needs_a_device_a_channel_and_a_known_stream() {
        let base = GridSpec {
            layout: "auto".to_string(),
            tiles: vec![tile("  gate  ", 1, "main")],
        };
        let normalised = base.normalised().expect("trims the device");
        assert_eq!(normalised.tiles[0].device, "gate");
        for broken in [
            GridTile {
                device: "   ".to_string(),
                ..tile("gate", 1, "main")
            },
            tile("gate", 0, "main"),
            tile("gate", 1, "medium"),
        ] {
            let spec = GridSpec {
                layout: "auto".to_string(),
                tiles: vec![broken.clone()],
            };
            assert!(spec.normalised().is_err(), "{broken:?} should not save");
        }
    }
}
