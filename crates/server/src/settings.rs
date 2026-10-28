use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};

const FILE: &str = "settings.json";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Secret,
    Number,
    Select,
    Toggle,
    Path,
}

pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub kind: Kind,
    pub options: &'static [&'static str],
    pub hint: &'static str,
    pub restart: bool,
}

const fn field(
    key: &'static str,
    label: &'static str,
    group: &'static str,
    kind: Kind,
    options: &'static [&'static str],
    hint: &'static str,
    restart: bool,
) -> Field {
    Field {
        key,
        label,
        group,
        kind,
        options,
        hint,
        restart,
    }
}

pub const FIELDS: &[Field] = &[
    field(
        "PROVIEW_PROVIDER",
        "Provider",
        "Cloud",
        Kind::Select,
        &["proview", "local", "fixture"],
        "Who answers device, stream and settings calls: the vendor cloud, this network, or built-in sample data.",
        true,
    ),
    field(
        "PROVIEW_MODE",
        "Default mode",
        "Cloud",
        Kind::Select,
        &["cloud", "local", "fixture"],
        "Mode a new session starts in.",
        false,
    ),
    field(
        "PROVIEW_CLOUD_BASE",
        "Cloud address",
        "Cloud",
        Kind::Text,
        &[],
        "Host of the signed cloud login envelope.",
        true,
    ),
    field(
        "PROVIEW_PORTAL_BASE",
        "Portal address",
        "Cloud",
        Kind::Text,
        &[],
        "Address used by the account login.",
        false,
    ),
    field(
        "PROVIEW_CAPS_BASE",
        "Capability address",
        "Cloud",
        Kind::Text,
        &[],
        "Address that answers device capabilities by serial.",
        false,
    ),
    field(
        "PROVIEW_CLOUD_SESSION_TTL",
        "Cloud session lifetime",
        "Cloud",
        Kind::Number,
        &[],
        "Seconds before a cloud session must sign in again.",
        false,
    ),
    field(
        "PROVIEW_DEVICE_HOST",
        "Camera address",
        "Local network",
        Kind::Text,
        &[],
        "Address and port of the camera or recorder, for example 192.168.0.100:34567.",
        true,
    ),
    field(
        "PROVIEW_DEVICE_RTSP",
        "Camera video address",
        "Local network",
        Kind::Text,
        &[],
        "Address and port that serves the video stream, usually 554.",
        true,
    ),
    field(
        "PROVIEW_DEVICE_ONVIF",
        "Camera control address",
        "Local network",
        Kind::Text,
        &[],
        "Address used for movement and presets.",
        true,
    ),
    field(
        "PROVIEW_DEVICE_SERIAL",
        "Camera serial for the device sign-in",
        "Local network",
        Kind::Text,
        &[],
        "Serial the camera user and password belong to. The cloud path only sends them to this serial.",
        true,
    ),
    field(
        "PROVIEW_DEVICE_USER",
        "Camera user",
        "Local network",
        Kind::Text,
        &[],
        "User name the camera accepts.",
        true,
    ),
    field(
        "PROVIEW_DEVICE_PASS",
        "Camera password",
        "Local network",
        Kind::Secret,
        &[],
        "Stored in the settings file and never shown back.",
        true,
    ),
    field(
        "PROVIEW_SESSION_TTL",
        "Sign-in lifetime",
        "Server",
        Kind::Number,
        &[],
        "Seconds a browser sign-in stays valid.",
        true,
    ),
    field(
        "PROVIEW_LISTEN",
        "Server address",
        "Server",
        Kind::Text,
        &[],
        "Address the standalone server listens on; the desktop app picks its own.",
        true,
    ),
    field(
        "PROVIEW_FFMPEG",
        "ffmpeg program",
        "Streaming",
        Kind::Path,
        &[],
        "Program used to convert HEVC video; empty uses the ffmpeg already on this machine.",
        false,
    ),
    field(
        "PROVIEW_LENS_PTZ",
        "Movement channel",
        "Streaming",
        Kind::Number,
        &[],
        "Channel that carries movement, when the camera reports one.",
        false,
    ),
    field(
        "PROVIEW_LENS_CAPABILITIES",
        "Lens features",
        "Streaming",
        Kind::Text,
        &[],
        "Comma separated list of lens features to advertise.",
        false,
    ),
    field(
        "PROVIEW_FIXTURE_DIR",
        "Sample data folder",
        "Advanced",
        Kind::Path,
        &[],
        "Folder of sample camera data, used only by the sample provider.",
        true,
    ),
    field(
        "PROVIEW_PROFILE",
        "Credential profile",
        "Advanced",
        Kind::Text,
        &[],
        "Name the desktop app uses for stored device credentials.",
        true,
    ),
    field(
        "PROVIEW_DATA_DIR",
        "Data folder",
        "Advanced",
        Kind::Path,
        &[],
        "Where the settings file and saved state live.",
        true,
    ),
    field(
        "ROCKET_SECRET_KEY",
        "Sign-in cookie secret",
        "Advanced",
        Kind::Secret,
        &[],
        "Signs the sign-in cookie; leave empty to let the app generate one.",
        true,
    ),
    field(
        "PROVIEW_APP_UUID",
        "Platform app uuid",
        "Self-built builds",
        Kind::Text,
        &[],
        "Only for a build made from source without a baked cloud identity; release builds carry one.",
        false,
    ),
    field(
        "PROVIEW_APP_KEY",
        "Platform app key",
        "Self-built builds",
        Kind::Text,
        &[],
        "Only for a build made from source without a baked cloud identity; release builds carry one.",
        false,
    ),
    field(
        "PROVIEW_APP_SECRET",
        "Platform app secret",
        "Self-built builds",
        Kind::Secret,
        &[],
        "Only for a build made from source without a baked cloud identity; release builds carry one.",
        false,
    ),
];

pub fn field_for(key: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|field| field.key == key)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SettingsFile {
    pub values: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldView {
    pub key: String,
    pub label: String,
    pub group: String,
    pub kind: String,
    pub options: Vec<String>,
    pub hint: String,
    pub restart: bool,
    pub secret: bool,
    pub set: bool,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub path: Option<String>,
    pub exists: bool,
    pub fields: Vec<FieldView>,
}

#[derive(Debug)]
pub struct SettingsStore {
    path: Option<PathBuf>,
    values: RwLock<BTreeMap<String, String>>,
    grid: RwLock<Option<serde_json::Value>>,
}

impl SettingsStore {
    pub fn load(data_dir: Option<&Path>) -> Self {
        let dir = data_dir.map(Path::to_path_buf).or_else(|| {
            env::var("PROVIEW_DATA_DIR")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        });
        let path = dir.map(|dir| dir.join(FILE));
        let file = path
            .as_deref()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|raw| serde_json::from_str::<SettingsFile>(&raw).ok())
            .unwrap_or_default();
        Self {
            path,
            values: RwLock::new(file.values),
            grid: RwLock::new(file.grid),
        }
    }

    pub fn grid_spec(&self) -> crate::routes::grid::GridSpec {
        self.grid
            .read()
            .expect("grid lock")
            .as_ref()
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default()
    }

    pub fn set_grid_spec(&self, spec: &crate::routes::grid::GridSpec) -> Result<(), String> {
        let value = serde_json::to_value(spec).map_err(|error| format!("encode grid: {error}"))?;
        let values = self.values.read().expect("settings lock");
        let mut grid = self.grid.write().expect("grid lock");
        self.persist(&values, Some(&value))?;
        *grid = Some(value);
        drop(grid);
        drop(values);
        Ok(())
    }

    fn persist(
        &self,
        values: &BTreeMap<String, String>,
        grid: Option<&serde_json::Value>,
    ) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("create directory: {error}"))?;
        }
        let staged = path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(&SettingsFile {
            values: values.clone(),
            grid: grid.cloned(),
        })
        .map_err(|error| format!("encode settings: {error}"))?;
        fs::write(&staged, body).map_err(|error| format!("write settings: {error}"))?;
        fs::rename(&staged, path).map_err(|error| format!("replace settings: {error}"))
    }

    pub fn apply_env(&self) {
        let values = self.values.read().expect("settings lock");
        for (key, value) in values.iter() {
            if value.trim().is_empty() || field_for(key).is_none() {
                continue;
            }
            if env::var_os(key).is_none() {
                env::set_var(key, value);
            }
        }
    }

    pub fn view(&self) -> SettingsView {
        let values = self.values.read().expect("settings lock");
        let fields = FIELDS
            .iter()
            .map(|field| {
                let secret = field.kind == Kind::Secret;
                let current = env::var(field.key)
                    .ok()
                    .filter(|value| !value.trim().is_empty());
                let stored = values
                    .get(field.key)
                    .filter(|value| !value.trim().is_empty());
                let is_set = current.is_some() || stored.is_some();
                let value = if secret {
                    None
                } else {
                    current.clone().or_else(|| stored.cloned())
                };
                FieldView {
                    key: field.key.to_string(),
                    label: field.label.to_string(),
                    group: field.group.to_string(),
                    kind: match field.kind {
                        Kind::Text => "text",
                        Kind::Secret => "secret",
                        Kind::Number => "number",
                        Kind::Select => "select",
                        Kind::Toggle => "toggle",
                        Kind::Path => "path",
                    }
                    .to_string(),
                    options: field.options.iter().map(|item| item.to_string()).collect(),
                    hint: field.hint.to_string(),
                    restart: field.restart,
                    secret,
                    set: is_set,
                    value,
                }
            })
            .collect();
        SettingsView {
            path: self.path.as_ref().map(|path| path.display().to_string()),
            exists: self.path.as_deref().is_some_and(Path::exists),
            fields,
        }
    }

    pub fn update(&self, incoming: BTreeMap<String, String>) -> Result<SettingsView, String> {
        for (key, value) in &incoming {
            let Some(field) = field_for(key) else {
                return Err(format!("unknown setting {key}"));
            };
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            match field.kind {
                Kind::Number => {
                    trimmed
                        .parse::<u64>()
                        .map_err(|_| format!("{} expects a whole number", field.label))?;
                }
                Kind::Select if !field.options.contains(&trimmed) => {
                    return Err(format!(
                        "{} must be one of {}",
                        field.label,
                        field.options.join(", ")
                    ));
                }
                _ => {}
            }
        }
        let mut guard = self.values.write().expect("settings lock");
        let grid = self.grid.read().expect("grid lock");
        let mut next = guard.clone();
        for (key, value) in incoming {
            if field_for(&key).is_none() {
                continue;
            }
            let trimmed = value.trim();
            if trimmed.is_empty() {
                next.remove(&key);
                if env::var_os(&key).is_some() {
                    env::remove_var(&key);
                }
            } else {
                next.insert(key.clone(), trimmed.to_string());
                env::set_var(&key, trimmed);
            }
        }
        self.persist(&next, grid.as_ref())?;
        *guard = next;
        drop(grid);
        drop(guard);
        Ok(self.view())
    }
}

#[derive(Serialize)]
pub struct StatusBody {
    pub ok: bool,
    pub message: String,
}

type Failure = (Status, Json<StatusBody>);

fn fail(status: Status, message: impl Into<String>) -> Failure {
    (
        status,
        Json(StatusBody {
            ok: false,
            message: message.into(),
        }),
    )
}

#[get("/settings")]
fn read_settings(store: &State<SettingsStore>) -> Json<SettingsView> {
    Json(store.view())
}

#[post("/settings", data = "<body>")]
fn write_settings(
    store: &State<SettingsStore>,
    body: Json<SettingsFile>,
) -> Result<Json<SettingsView>, Failure> {
    store
        .inner()
        .update(body.into_inner().values)
        .map(Json)
        .map_err(|error| fail(Status::BadRequest, error))
}

pub fn routes() -> Vec<rocket::route::Route> {
    routes![read_settings, write_settings]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> (SettingsStore, PathBuf) {
        let dir = env::temp_dir().join(format!("proview-settings-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        (SettingsStore::load(Some(&dir)), dir)
    }

    #[test]
    fn the_view_says_whether_a_settings_file_exists() {
        let (store, dir) = temp_store("exists");
        assert!(!store.view().exists, "nothing is written yet");
        let mut values = BTreeMap::new();
        values.insert("PROVIEW_DEVICE_USER".to_string(), "admin".to_string());
        store.update(values).expect("update");
        assert!(store.view().exists, "the save wrote the file");
        env::remove_var("PROVIEW_DEVICE_USER");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_unknown_keys_and_bad_numbers() {
        let (store, dir) = temp_store("unknown");
        let mut unknown = BTreeMap::new();
        unknown.insert("PROVIEW_NOPE".to_string(), "x".to_string());
        assert!(store.update(unknown).is_err());
        let mut bad = BTreeMap::new();
        bad.insert("PROVIEW_SESSION_TTL".to_string(), "soon".to_string());
        assert!(store.update(bad).is_err());
        let mut bad_select = BTreeMap::new();
        bad_select.insert("PROVIEW_PROVIDER".to_string(), "carrier-pigeon".to_string());
        assert!(store.update(bad_select).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn persists_and_hides_secrets() {
        let (store, dir) = temp_store("secrets");
        let mut values = BTreeMap::new();
        values.insert("PROVIEW_DEVICE_USER".to_string(), "admin".to_string());
        values.insert("PROVIEW_DEVICE_PASS".to_string(), "hunter2".to_string());
        let view = store.update(values).expect("update");
        let pass = view
            .fields
            .iter()
            .find(|field| field.key == "PROVIEW_DEVICE_PASS")
            .expect("password field");
        assert!(pass.secret);
        assert!(pass.set);
        assert_eq!(pass.value, None);
        let user = view
            .fields
            .iter()
            .find(|field| field.key == "PROVIEW_DEVICE_USER")
            .expect("user field");
        assert_eq!(user.value.as_deref(), Some("admin"));
        let raw = fs::read_to_string(dir.join(FILE)).expect("settings file");
        assert!(raw.contains("hunter2"));
        env::remove_var("PROVIEW_DEVICE_USER");
        env::remove_var("PROVIEW_DEVICE_PASS");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_second_save_keeps_the_first() {
        let (store, dir) = temp_store("merge");
        let mut first = BTreeMap::new();
        first.insert(
            "PROVIEW_DEVICE_HOST".to_string(),
            "192.168.0.100:34567".to_string(),
        );
        store.update(first).expect("first save");
        let mut second = BTreeMap::new();
        second.insert(
            "PROVIEW_CLOUD_BASE".to_string(),
            "https://cloud.example".to_string(),
        );
        store.update(second).expect("second save");
        let raw = fs::read_to_string(dir.join(FILE)).expect("settings file");
        assert!(
            raw.contains("PROVIEW_DEVICE_HOST"),
            "the second save dropped the first: {raw}"
        );
        assert!(raw.contains("PROVIEW_CLOUD_BASE"));
        let view = store.view();
        let host = view
            .fields
            .iter()
            .find(|f| f.key == "PROVIEW_DEVICE_HOST")
            .expect("host field");
        assert_eq!(host.value.as_deref(), Some("192.168.0.100:34567"));
        env::remove_var("PROVIEW_DEVICE_HOST");
        env::remove_var("PROVIEW_CLOUD_BASE");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fields_read_once_at_startup_declare_a_restart() {
        let restart: Vec<&str> = FIELDS.iter().filter(|f| f.restart).map(|f| f.key).collect();
        for key in [
            "PROVIEW_DEVICE_HOST",
            "PROVIEW_DEVICE_RTSP",
            "PROVIEW_DEVICE_ONVIF",
            "PROVIEW_DEVICE_USER",
            "PROVIEW_DEVICE_PASS",
            "PROVIEW_SESSION_TTL",
            "PROVIEW_FIXTURE_DIR",
            "PROVIEW_PROFILE",
            "PROVIEW_PROVIDER",
            "PROVIEW_CLOUD_BASE",
        ] {
            assert!(
                restart.contains(&key),
                "{key} feeds startup config and must say it needs a restart"
            );
        }
        for key in [
            "PROVIEW_MODE",
            "PROVIEW_PORTAL_BASE",
            "PROVIEW_CAPS_BASE",
            "PROVIEW_CLOUD_SESSION_TTL",
            "PROVIEW_APP_UUID",
            "PROVIEW_APP_KEY",
            "PROVIEW_APP_SECRET",
        ] {
            assert!(
                !restart.contains(&key),
                "{key} is read per request and must not claim a restart"
            );
        }
    }

    #[test]
    fn the_platform_identity_sits_under_self_built_builds() {
        for key in ["PROVIEW_APP_UUID", "PROVIEW_APP_KEY", "PROVIEW_APP_SECRET"] {
            let field = field_for(key).expect(key);
            assert_eq!(field.group, "Self-built builds", "{key}");
        }
    }

    #[test]
    fn clears_a_value() {
        let (store, dir) = temp_store("clear");
        let mut values = BTreeMap::new();
        values.insert("PROVIEW_FFMPEG".to_string(), "/opt/ffmpeg".to_string());
        store.update(values).expect("update");
        let mut clear = BTreeMap::new();
        clear.insert("PROVIEW_FFMPEG".to_string(), String::new());
        let view = store.update(clear).expect("clear");
        let field = view
            .fields
            .iter()
            .find(|field| field.key == "PROVIEW_FFMPEG")
            .expect("ffmpeg field");
        assert!(!field.set);
        env::remove_var("PROVIEW_FFMPEG");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_grid_save_and_a_settings_save_do_not_collide() {
        let dir = env::temp_dir().join(format!("proview-settings-race-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let store = std::sync::Arc::new(SettingsStore::load(Some(&dir)));
        let mut workers = Vec::new();
        for worker in 0..4 {
            let store = store.clone();
            workers.push(std::thread::spawn(move || {
                for turn in 0..25 {
                    let spec = crate::routes::grid::GridSpec {
                        layout: "auto".to_string(),
                        tiles: vec![crate::routes::grid::GridTile {
                            device: format!("cam-{worker}-{turn}"),
                            channel: 1,
                            quality: "main".to_string(),
                        }],
                    };
                    store.set_grid_spec(&spec).expect("grid save");
                    store.update(BTreeMap::new()).expect("settings save");
                }
            }));
        }
        for worker in workers {
            worker.join().expect("worker");
        }
        let raw = fs::read_to_string(dir.join(FILE)).expect("settings file");
        let file: SettingsFile = serde_json::from_str(&raw).expect("settings json");
        assert!(file.grid.is_some(), "the grid stayed in the file");
        assert_eq!(store.grid_spec().tiles.len(), 1);
        let _ = fs::remove_dir_all(dir);
    }
}
