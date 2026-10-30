use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};

use crate::desktop::{DesktopControl, OfferedToken, PageHost, PageOrigin, PagePeer};

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
        &["proview", "fixture"],
        "Who answers device, stream and settings calls: the camera, through the cloud or local mode, or built-in sample data.",
        true,
    ),
    field(
        "PROVIEW_MODE",
        "Default mode",
        "Cloud",
        Kind::Select,
        &["cloud", "local"],
        "Mode the server starts in until a mode is chosen in the app.",
        true,
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
        true,
    ),
    field(
        "PROVIEW_CAPS_BASE",
        "Capability address",
        "Cloud",
        Kind::Text,
        &[],
        "Address that answers device capabilities by serial.",
        true,
    ),
    field(
        "PROVIEW_CLOUD_SESSION_TTL",
        "Cloud session lifetime",
        "Cloud",
        Kind::Number,
        &[],
        "Seconds before a cloud session must sign in again.",
        true,
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
        "Address the standalone server listens on; --listen and PROVIEW_LISTEN in the environment win over the saved value, and the desktop app picks its own.",
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
        "Signs the sign-in cookie: 256-bit base64, e.g. from openssl rand -base64 32. A release server refuses to start without one; the environment wins over the saved value.",
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
    field(
        "PROVIEW_DESKTOP_ALARM_NOTIFY",
        "Alarm notifications",
        "Desktop",
        Kind::Toggle,
        &[],
        "Raise one system notification per new alarm while the desktop app is running.",
        false,
    ),
    field(
        "PROVIEW_DESKTOP_ALARM_INTERVAL",
        "Alarm check interval",
        "Desktop",
        Kind::Number,
        &[],
        "Seconds between alarm checks while the desktop app runs, 15 to 3600; 60 when empty.",
        false,
    ),
    field(
        "PROVIEW_DESKTOP_CLOSE_TO_TRAY",
        "Keep running when the window closes",
        "Desktop",
        Kind::Toggle,
        &[],
        "Closing the window leaves the app in the system tray instead of quitting it.",
        false,
    ),
    field(
        "PROVIEW_DESKTOP_MINIMIZE_TO_TRAY",
        "Hide to tray when minimized",
        "Desktop",
        Kind::Toggle,
        &[],
        "Minimizing takes the window off the taskbar and leaves the tray icon.",
        false,
    ),
];

pub fn default_of(key: &str) -> Option<&'static str> {
    match key {
        "PROVIEW_DESKTOP_ALARM_NOTIFY" => Some("off"),
        "PROVIEW_DESKTOP_ALARM_INTERVAL" => Some("60"),
        "PROVIEW_DESKTOP_CLOSE_TO_TRAY" => Some("on"),
        "PROVIEW_DESKTOP_MINIMIZE_TO_TRAY" => Some("off"),
        _ => None,
    }
}

pub fn bounds_of(key: &str) -> Option<(u64, u64)> {
    match key {
        "PROVIEW_DESKTOP_ALARM_INTERVAL" => Some((15, 3600)),
        _ => None,
    }
}

const DESKTOP_IGNORES: &[&str] = &["ROCKET_SECRET_KEY"];

pub fn shown_on(key: &str, desktop: bool) -> bool {
    !(desktop && DESKTOP_IGNORES.contains(&key))
}

fn exports_env(key: &str) -> bool {
    !key.starts_with("PROVIEW_DESKTOP_")
}

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
    pub default: Option<String>,
    pub min: Option<u64>,
    pub max: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub path: Option<String>,
    pub exists: bool,
    pub fields: Vec<FieldView>,
}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: Option<PathBuf>,
    values: Arc<RwLock<BTreeMap<String, String>>>,
    grid: Arc<RwLock<Option<serde_json::Value>>>,
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
            values: Arc::new(RwLock::new(file.values)),
            grid: Arc::new(RwLock::new(file.grid)),
        }
    }

    pub fn value(&self, key: &str) -> Option<String> {
        let values = self.values.read().expect("settings lock");
        values
            .get(key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
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
            if value.trim().is_empty() || field_for(key).is_none() || !exports_env(key) {
                continue;
            }
            if env::var_os(key).is_none() {
                env::set_var(key, value);
            }
        }
    }

    pub fn view(&self, desktop: bool) -> SettingsView {
        let values = self.values.read().expect("settings lock");
        let fields = FIELDS
            .iter()
            .filter(|field| shown_on(field.key, desktop))
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
                    default: default_of(field.key).map(str::to_string),
                    min: bounds_of(field.key).map(|(low, _)| low),
                    max: bounds_of(field.key).map(|(_, high)| high),
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
                    let number = trimmed
                        .parse::<u64>()
                        .map_err(|_| format!("{} expects a whole number", field.label))?;
                    if let Some((low, high)) = bounds_of(key) {
                        if number < low || number > high {
                            return Err(format!(
                                "{} must be between {low} and {high}",
                                field.label
                            ));
                        }
                    }
                }
                Kind::Select if !field.options.contains(&trimmed) => {
                    return Err(format!(
                        "{} must be one of {}",
                        field.label,
                        field.options.join(", ")
                    ));
                }
                Kind::Toggle if trimmed != "on" && trimmed != "off" => {
                    return Err(format!("{} is either on or off", field.label));
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
                if exports_env(&key) && env::var_os(&key).is_some() {
                    env::remove_var(&key);
                }
            } else {
                next.insert(key.clone(), trimmed.to_string());
                if exports_env(&key) {
                    env::set_var(&key, trimmed);
                }
            }
        }
        self.persist(&next, grid.as_ref())?;
        *guard = next;
        drop(grid);
        drop(guard);
        Ok(self.view(false))
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

fn loopback_peer(peer: Option<SocketAddr>) -> bool {
    peer.is_some_and(|address| address.ip().to_canonical().is_loopback())
}

fn loopback_authority(host: &str) -> bool {
    let name = match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    };
    if name.eq_ignore_ascii_case("localhost") || name == "[::1]" {
        return true;
    }
    name.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

const SESSION_COOKIE: &str = "pvw_session";

fn gate(
    control: Option<&DesktopControl>,
    host: &str,
    origin: Option<&str>,
    offered: &str,
    cookies: &CookieJar<'_>,
    sessions: &Store,
    peer: Option<SocketAddr>,
) -> Result<(), Failure> {
    match control {
        Some(control) => {
            if !control.serves(host) {
                return Err(fail(
                    Status::Forbidden,
                    format!(
                        "{host} is not the address this app opened, and the settings answer only on {}",
                        control.authority()
                    ),
                ));
            }
            if let Some(origin) = origin {
                if !crate::desktop::same_origin(origin, control.authority()) {
                    return Err(fail(
                        Status::Forbidden,
                        format!(
                            "{origin} is not the page this app serves, and the settings answer only to {}",
                            control.authority()
                        ),
                    ));
                }
            }
            if !offered.is_empty() && control.accepts(offered) {
                return Ok(());
            }
        }
        None => {
            if !loopback_peer(peer) {
                return Err(fail(
                    Status::Forbidden,
                    "this request did not arrive from this machine, and the settings answer only there",
                ));
            }
            if let Some(origin) = origin {
                if !crate::desktop::same_origin(origin, host) {
                    return Err(fail(
                        Status::Forbidden,
                        format!("{origin} is not this server's own origin ({host})"),
                    ));
                }
            }
            if !loopback_authority(host) {
                return Err(fail(
                    Status::Forbidden,
                    format!("{host} is not this machine, and the settings answer only on a loopback address"),
                ));
            }
            return Ok(());
        }
    }
    let id = cookies
        .get_private(SESSION_COOKIE)
        .map(|cookie| cookie.value().to_string())
        .ok_or_else(|| {
            fail(
                Status::Unauthorized,
                "the settings are only for a signed-in page; sign in first",
            )
        })?;
    sessions.get(&id).map(|_| ()).ok_or_else(|| {
        fail(
            Status::Unauthorized,
            "the session expired; sign in again to read or change the settings",
        )
    })
}

#[get("/settings")]
#[allow(clippy::too_many_arguments)]
fn read_settings(
    settings: &State<SettingsStore>,
    sessions: &State<Store>,
    control: &State<Option<DesktopControl>>,
    host: PageHost,
    origin: PageOrigin,
    offered: OfferedToken,
    peer: PagePeer,
    cookies: &CookieJar<'_>,
) -> Result<Json<SettingsView>, Failure> {
    gate(
        control.inner().as_ref(),
        &host.0,
        origin.0.as_deref(),
        &offered.0,
        cookies,
        sessions.inner(),
        peer.0,
    )?;
    Ok(Json(settings.view(control.is_some())))
}

#[post("/settings", data = "<body>")]
#[allow(clippy::too_many_arguments)]
fn write_settings(
    settings: &State<SettingsStore>,
    sessions: &State<Store>,
    control: &State<Option<DesktopControl>>,
    host: PageHost,
    origin: PageOrigin,
    offered: OfferedToken,
    peer: PagePeer,
    cookies: &CookieJar<'_>,
    body: Json<SettingsFile>,
) -> Result<Json<SettingsView>, Failure> {
    gate(
        control.inner().as_ref(),
        &host.0,
        origin.0.as_deref(),
        &offered.0,
        cookies,
        sessions.inner(),
        peer.0,
    )?;
    let desktop = control.is_some();
    let values = body.into_inner().values;
    if let Some(key) = values.keys().find(|key| !shown_on(key, desktop)) {
        return Err(fail(
            Status::BadRequest,
            format!("{key} is not used by the desktop app"),
        ));
    }
    settings
        .inner()
        .update(values)
        .map(|_| Json(settings.view(desktop)))
        .map_err(|error| fail(Status::BadRequest, error))
}

pub fn routes() -> Vec<rocket::route::Route> {
    routes![read_settings, write_settings]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocket::http::{ContentType, Header};
    use rocket::local::blocking::Client;

    const LOCAL: ([u8; 4], u16) = ([127, 0, 0, 1], 40000);
    const REMOTE: ([u8; 4], u16) = ([192, 168, 1, 50], 40000);

    fn temp_store(tag: &str) -> (SettingsStore, PathBuf) {
        let dir = env::temp_dir().join(format!("proview-settings-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        (SettingsStore::load(Some(&dir)), dir)
    }

    #[test]
    fn the_view_says_whether_a_settings_file_exists() {
        let (store, dir) = temp_store("exists");
        assert!(!store.view(false).exists, "nothing is written yet");
        let mut values = BTreeMap::new();
        values.insert("PROVIEW_DEVICE_USER".to_string(), "admin".to_string());
        store.update(values).expect("update");
        assert!(store.view(false).exists, "the save wrote the file");
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
        let view = store.view(false);
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
            "PROVIEW_MODE",
            "PROVIEW_PORTAL_BASE",
            "PROVIEW_CAPS_BASE",
            "PROVIEW_CLOUD_SESSION_TTL",
            "PROVIEW_LISTEN",
            "ROCKET_SECRET_KEY",
        ] {
            assert!(
                restart.contains(&key),
                "{key} feeds startup config and must say it needs a restart"
            );
        }
        for key in ["PROVIEW_APP_UUID", "PROVIEW_APP_KEY", "PROVIEW_APP_SECRET"] {
            assert!(
                !restart.contains(&key),
                "{key} is read per request and must not claim a restart"
            );
        }
    }

    #[test]
    fn selects_offer_only_values_the_server_acts_on() {
        let provider = field_for("PROVIEW_PROVIDER").expect("provider");
        assert_eq!(provider.options, ["proview", "fixture"]);
        let mode = field_for("PROVIEW_MODE").expect("mode");
        assert_eq!(mode.options, ["cloud", "local"]);
        for raw in mode.options {
            assert!(
                proview_core::provider::Mode::parse(raw).is_some(),
                "{raw} is a mode the server parses"
            );
        }
        let (store, dir) = temp_store("selects");
        for (key, value) in [("PROVIEW_PROVIDER", "local"), ("PROVIEW_MODE", "fixture")] {
            let mut values = BTreeMap::new();
            values.insert(key.to_string(), value.to_string());
            assert!(store.update(values).is_err(), "{key}={value} is refused");
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_desktop_hides_the_cookie_secret_it_never_reads() {
        let desk = tower("secret-desk", Some("127.0.0.1:8787"));
        let token = Some("desk-token-0123456789");
        let (status, body) = desk.get("127.0.0.1:8787", None, token);
        assert_eq!(status, Status::Ok, "{body}");
        assert!(body.contains("PROVIEW_DEVICE_PASS"), "{body}");
        assert!(!body.contains("ROCKET_SECRET_KEY"), "{body}");
        let (status, body) = desk.save_body(
            "127.0.0.1:8787",
            Some("http://127.0.0.1:8787"),
            token,
            r#"{"values":{"ROCKET_SECRET_KEY":"aGVsbG8gd29ybGQgdGhpcyBpcyBhIHRlc3Qga2V5ISE="}}"#,
        );
        assert_eq!(status, Status::BadRequest, "{body}");
        assert!(body.contains("not used by the desktop app"), "{body}");
        assert!(
            !desk.data.join(FILE).exists(),
            "the refused save wrote nothing"
        );
        let server = tower("secret-server", None);
        let (status, body) = server.get("127.0.0.1:8787", None, None);
        assert_eq!(status, Status::Ok, "{body}");
        assert!(body.contains("ROCKET_SECRET_KEY"), "{body}");
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
    fn the_desktop_keys_carry_a_group_and_a_default() {
        for key in [
            "PROVIEW_DESKTOP_ALARM_NOTIFY",
            "PROVIEW_DESKTOP_ALARM_INTERVAL",
            "PROVIEW_DESKTOP_CLOSE_TO_TRAY",
            "PROVIEW_DESKTOP_MINIMIZE_TO_TRAY",
        ] {
            let field = field_for(key).expect(key);
            assert_eq!(field.group, "Desktop", "{key}");
            assert!(default_of(key).is_some(), "{key} needs a default");
        }
        assert_eq!(default_of("PROVIEW_DESKTOP_CLOSE_TO_TRAY"), Some("on"));
        assert_eq!(default_of("PROVIEW_DESKTOP_MINIMIZE_TO_TRAY"), Some("off"));
        assert_eq!(default_of("PROVIEW_DESKTOP_ALARM_INTERVAL"), Some("60"));
        assert_eq!(default_of("PROVIEW_DESKTOP_ALARM_NOTIFY"), Some("off"));
    }

    #[test]
    fn desktop_toggles_take_on_or_off_and_the_interval_stays_in_range() {
        let (store, dir) = temp_store("desktop");
        let mut bad_toggle = BTreeMap::new();
        bad_toggle.insert(
            "PROVIEW_DESKTOP_CLOSE_TO_TRAY".to_string(),
            "maybe".to_string(),
        );
        assert!(store.update(bad_toggle).is_err());
        let mut too_fast = BTreeMap::new();
        too_fast.insert(
            "PROVIEW_DESKTOP_ALARM_INTERVAL".to_string(),
            "2".to_string(),
        );
        assert!(store.update(too_fast).is_err());
        let mut too_slow = BTreeMap::new();
        too_slow.insert(
            "PROVIEW_DESKTOP_ALARM_INTERVAL".to_string(),
            "99999".to_string(),
        );
        assert!(store.update(too_slow).is_err());
        let mut good = BTreeMap::new();
        good.insert("PROVIEW_DESKTOP_ALARM_NOTIFY".to_string(), "on".to_string());
        good.insert(
            "PROVIEW_DESKTOP_ALARM_INTERVAL".to_string(),
            "90".to_string(),
        );
        store.update(good).expect("the desktop values save");
        assert_eq!(
            store.value("PROVIEW_DESKTOP_ALARM_NOTIFY").as_deref(),
            Some("on")
        );
        assert_eq!(
            store.value("PROVIEW_DESKTOP_ALARM_INTERVAL").as_deref(),
            Some("90")
        );
        let view = store.view(false);
        let interval = view
            .fields
            .iter()
            .find(|field| field.key == "PROVIEW_DESKTOP_ALARM_INTERVAL")
            .expect("interval field");
        assert_eq!(interval.min, Some(15));
        assert_eq!(interval.max, Some(3600));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_clone_reads_the_same_values_as_the_original() {
        let (store, dir) = temp_store("clone");
        let shared = store.clone();
        let mut values = BTreeMap::new();
        values.insert("PROVIEW_DESKTOP_ALARM_NOTIFY".to_string(), "on".to_string());
        store.update(values).expect("update");
        assert_eq!(
            shared.value("PROVIEW_DESKTOP_ALARM_NOTIFY").as_deref(),
            Some("on"),
            "the desktop shell holds a clone of the store the server writes"
        );
        let _ = fs::remove_dir_all(dir);
    }

    static SLOT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    fn scratch(tag: &str) -> PathBuf {
        let slot = SLOT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("proview-gate-{}-{slot}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn fixtures(tag: &str) -> PathBuf {
        let dir = scratch(tag);
        fs::write(
            dir.join("account.json"),
            r#"{"email":"fixture@example.invalid","password":"fixture-password"}"#,
        )
        .expect("account fixture");
        fs::write(
            dir.join("devices.json"),
            r#"[{"id":"gate","name":"Gate camera","model":"XM-IPC","online":true,"channels":1,"lan_host":null}]"#,
        )
        .expect("device fixture");
        dir
    }

    fn fixture_config(dir: &std::path::Path) -> proview_core::config::Config {
        proview_core::config::Config {
            provider: proview_core::config::ProviderKind::Fixture,
            cloud_base: String::new(),
            share_base: proview_core::cloud::attributes::SHARE_BASE.to_string(),
            device_user: "admin".to_string(),
            device_pass: String::new(),
            fixture_dir: dir.to_string_lossy().to_string(),
            device_onvif: None,
            device_host: None,
            device_rtsp: None,
            session_ttl_secs: 600,
        }
    }

    struct Tower {
        client: Client,
        data: PathBuf,
        fixtures: PathBuf,
        _restarts: Option<tokio::sync::mpsc::UnboundedReceiver<()>>,
    }

    impl Tower {
        fn sign_in(&self) -> Status {
            self.client
                .post("/api/login")
                .header(ContentType::JSON)
                .body(r#"{"email":"fixture@example.invalid","password":"fixture-password"}"#)
                .dispatch()
                .status()
        }

        fn get(&self, host: &str, origin: Option<&str>, token: Option<&str>) -> (Status, String) {
            self.get_from(host, origin, token, LOCAL)
        }

        fn get_from(
            &self,
            host: &str,
            origin: Option<&str>,
            token: Option<&str>,
            peer: ([u8; 4], u16),
        ) -> (Status, String) {
            let mut request = self
                .client
                .get("/api/settings")
                .remote(peer.into())
                .header(Header::new("Host", host.to_string()));
            if let Some(origin) = origin {
                request = request.header(Header::new("Origin", origin.to_string()));
            }
            if let Some(token) = token {
                request = request.header(Header::new("x-proview-desktop-token", token.to_string()));
            }
            let response = request.dispatch();
            let status = response.status();
            (status, response.into_string().unwrap_or_default())
        }

        fn save(&self, host: &str, origin: Option<&str>, token: Option<&str>) -> (Status, String) {
            self.save_body(
                host,
                origin,
                token,
                r#"{"values":{"PROVIEW_DESKTOP_ALARM_INTERVAL":"90"}}"#,
            )
        }

        fn save_body(
            &self,
            host: &str,
            origin: Option<&str>,
            token: Option<&str>,
            body: &str,
        ) -> (Status, String) {
            let mut request = self
                .client
                .post("/api/settings")
                .remote(LOCAL.into())
                .header(Header::new("Host", host.to_string()))
                .header(ContentType::JSON)
                .body(body);
            if let Some(origin) = origin {
                request = request.header(Header::new("Origin", origin.to_string()));
            }
            if let Some(token) = token {
                request = request.header(Header::new("x-proview-desktop-token", token.to_string()));
            }
            let response = request.dispatch();
            let status = response.status();
            (status, response.into_string().unwrap_or_default())
        }
    }

    impl Drop for Tower {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.data);
            let _ = fs::remove_dir_all(&self.fixtures);
        }
    }

    fn tower(tag: &str, authority: Option<&str>) -> Tower {
        let data = scratch(&format!("{tag}-data"));
        let fixtures = fixtures(&format!("{tag}-fixtures"));
        let settings = SettingsStore::load(Some(&data));
        let figment = rocket::Config::figment()
            .merge(("secret_key", "aGVsbG8gd29ybGQgdGhpcyBpcyBhIHRlc3Qga2V5ISE="));
        let (control, restarts) = match authority {
            Some(authority) => {
                let (control, receiver) = DesktopControl::new("desk-token-0123456789");
                (Some(control.for_authority(authority)), Some(receiver))
            }
            None => (None, None),
        };
        let rocket = crate::app::build_with_control(
            figment,
            fixture_config(&fixtures),
            crate::config::ModeStore::new(proview_core::provider::Mode::default(), None),
            settings,
            crate::routes::cloud_bind::AccountStore::from_env(),
            control,
            Store::new(),
        );
        Tower {
            client: Client::tracked(rocket).expect("rocket"),
            data,
            fixtures,
            _restarts: restarts,
        }
    }

    #[test]
    fn a_foreign_host_is_refused_before_the_session_is_even_asked_for() {
        let app = tower("host", Some("127.0.0.1:8787"));
        let (status, body) = app.get("evil.example:8787", None, None);
        assert_eq!(status, Status::Forbidden, "{body}");
        assert!(
            body.contains("127.0.0.1:8787"),
            "the refusal names the only address that answers: {body}"
        );
        assert_eq!(
            app.sign_in(),
            Status::Ok,
            "the fixture sign-in gives a real session"
        );
        let (status, body) = app.get("evil.example:8787", None, None);
        assert_eq!(
            status,
            Status::Forbidden,
            "a session does not buy a foreign host any access: {body}"
        );
    }

    #[test]
    fn a_signed_in_page_on_the_app_address_reads_and_writes_the_settings() {
        let app = tower("session", Some("127.0.0.1:8787"));
        let (status, body) = app.get("127.0.0.1:8787", None, None);
        assert_eq!(status, Status::Unauthorized, "{body}");
        assert_eq!(app.sign_in(), Status::Ok);
        let (status, body) = app.get("127.0.0.1:8787", None, None);
        assert_eq!(status, Status::Ok, "{body}");
        assert!(
            body.contains("PROVIEW_DEVICE_PASS"),
            "the view is the settings view"
        );
        let (status, body) = app.save("127.0.0.1:8787", Some("http://127.0.0.1:8787"), None);
        assert_eq!(status, Status::Ok, "{body}");
        assert!(body.contains(r#""value":"90""#), "{body}");
    }

    #[test]
    fn the_desktop_token_stands_in_for_the_session_before_a_sign_in() {
        let app = tower("token", Some("127.0.0.1:8787"));
        let (status, body) = app.get("127.0.0.1:8787", None, Some("desk-token-0123456789"));
        assert_eq!(
            status,
            Status::Ok,
            "the page the app opened can read before sign-in: {body}"
        );
        let (status, body) = app.get("127.0.0.1:8787", None, Some("desk-token-0000000000"));
        assert_eq!(
            status,
            Status::Unauthorized,
            "a wrong token is no credential: {body}"
        );
        let (status, body) = app.get("127.0.0.1:8787", None, None);
        assert_eq!(status, Status::Unauthorized, "{body}");
    }

    #[test]
    fn another_origin_cannot_post_even_with_the_desktop_token() {
        let app = tower("origin", Some("127.0.0.1:8787"));
        let (status, body) = app.save(
            "127.0.0.1:8787",
            Some("https://evil.example"),
            Some("desk-token-0123456789"),
        );
        assert_eq!(status, Status::Forbidden, "{body}");
        let (status, body) = app.save(
            "127.0.0.1:8787",
            Some("http://127.0.0.1:8787"),
            Some("desk-token-0123456789"),
        );
        assert_eq!(status, Status::Ok, "{body}");
    }

    #[test]
    fn the_standalone_server_answers_this_machine_and_nothing_else() {
        let app = tower("standalone", None);
        let (status, body) = app.get("192.168.1.5:8787", None, None);
        assert_eq!(
            status,
            Status::Forbidden,
            "a request that arrived from the network is refused: {body}"
        );
        let (status, body) = app.get("127.0.0.1:8787", Some("https://evil.example"), None);
        assert_eq!(status, Status::Forbidden, "{body}");
        let (status, body) = app.get_from("127.0.0.1:8787", None, None, REMOTE);
        assert_eq!(
            status,
            Status::Forbidden,
            "a loopback Host forged by a client on the network is still refused: {body}"
        );
        let (status, body) = app.get("127.0.0.1:8787", None, None);
        assert_eq!(
            status,
            Status::Ok,
            "the sign-in screen keeps its settings so a wrong address can be fixed there: {body}"
        );
        assert_eq!(app.sign_in(), Status::Ok);
        let (status, body) = app.get("localhost:8787", Some("http://localhost:8787"), None);
        assert_eq!(status, Status::Ok, "{body}");
    }

    #[test]
    fn a_loopback_host_is_recognised_with_and_without_a_port() {
        for host in [
            "127.0.0.1:8787",
            "127.0.0.1",
            "localhost:8787",
            "LOCALHOST",
            "[::1]:8787",
            "[::1]",
            "127.9.9.9:80",
        ] {
            assert!(loopback_authority(host), "{host} is this machine");
        }
        for host in [
            "192.168.1.5:8787",
            "10.0.0.1:8787",
            "evil.example",
            "evil.example:8787",
            "0.0.0.0:8787",
            "[2001:db8::1]:8787",
            "",
        ] {
            assert!(!loopback_authority(host), "{host} is somewhere else");
        }
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
