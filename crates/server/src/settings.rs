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
        "Which provider answers device, stream and setting calls.",
        true,
    ),
    field(
        "PROVIEW_MODE",
        "Default mode",
        "Cloud",
        Kind::Select,
        &["cloud", "local", "fixture"],
        "Mode used when a session has not chosen one.",
        false,
    ),
    field(
        "PROVIEW_CLOUD_BASE",
        "Cloud base",
        "Cloud",
        Kind::Text,
        &[],
        "Vendor cloud host for the app API.",
        true,
    ),
    field(
        "PROVIEW_PORTAL_BASE",
        "Portal base",
        "Cloud",
        Kind::Text,
        &[],
        "Portal host used by the account login.",
        false,
    ),
    field(
        "PROVIEW_CAPS_BASE",
        "Caps base",
        "Cloud",
        Kind::Text,
        &[],
        "Host that answers device capabilities by serial.",
        false,
    ),
    field(
        "PROVIEW_CLOUD_SESSION_TTL",
        "Cloud session seconds",
        "Cloud",
        Kind::Number,
        &[],
        "How long a cloud session is kept before it must sign in again.",
        false,
    ),
    field(
        "PROVIEW_DEVICE_HOST",
        "Device host",
        "Local network",
        Kind::Text,
        &[],
        "DVRIP host and port, for example 192.168.0.100:34567.",
        true,
    ),
    field(
        "PROVIEW_DEVICE_RTSP",
        "Device RTSP",
        "Local network",
        Kind::Text,
        &[],
        "RTSP host and port, usually 554.",
        true,
    ),
    field(
        "PROVIEW_DEVICE_ONVIF",
        "Device ONVIF",
        "Local network",
        Kind::Text,
        &[],
        "ONVIF base address for movement and presets.",
        true,
    ),
    field(
        "PROVIEW_DEVICE_USER",
        "Device user",
        "Local network",
        Kind::Text,
        &[],
        "Camera user for LAN access.",
        true,
    ),
    field(
        "PROVIEW_DEVICE_PASS",
        "Device password",
        "Local network",
        Kind::Secret,
        &[],
        "Camera password for LAN access; stored in the settings file.",
        true,
    ),
    field(
        "PROVIEW_SESSION_TTL",
        "Session seconds",
        "Server",
        Kind::Number,
        &[],
        "Browser session lifetime.",
        true,
    ),
    field(
        "PROVIEW_LISTEN",
        "Listen address",
        "Server",
        Kind::Text,
        &[],
        "Address the server binds; only used by the standalone server.",
        true,
    ),
    field(
        "PROVIEW_FFMPEG",
        "ffmpeg path",
        "Streaming",
        Kind::Path,
        &[],
        "Binary used to transcode HEVC; empty means ffmpeg on PATH.",
        false,
    ),
    field(
        "PROVIEW_LENS_PTZ",
        "Lens PTZ channel",
        "Streaming",
        Kind::Number,
        &[],
        "Channel that carries movement, when the device reports one.",
        false,
    ),
    field(
        "PROVIEW_LENS_CAPABILITIES",
        "Lens capabilities",
        "Streaming",
        Kind::Text,
        &[],
        "Comma list of lens capabilities to advertise.",
        false,
    ),
    field(
        "PROVIEW_FIXTURE_DIR",
        "Fixture directory",
        "Advanced",
        Kind::Path,
        &[],
        "Directory the fixture provider reads from.",
        true,
    ),
    field(
        "PROVIEW_PROFILE",
        "Keychain profile",
        "Advanced",
        Kind::Text,
        &[],
        "Profile name the desktop uses for stored device credentials.",
        true,
    ),
    field(
        "PROVIEW_DATA_DIR",
        "Data directory",
        "Advanced",
        Kind::Path,
        &[],
        "Where the settings file and persisted state live.",
        true,
    ),
    field(
        "ROCKET_SECRET_KEY",
        "Cookie secret",
        "Advanced",
        Kind::Secret,
        &[],
        "Secret that signs the session cookie; empty lets the app generate one.",
        true,
    ),
];

pub fn field_for(key: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|field| field.key == key)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SettingsFile {
    pub values: BTreeMap<String, String>,
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
    pub fields: Vec<FieldView>,
}

#[derive(Debug)]
pub struct SettingsStore {
    path: Option<PathBuf>,
    values: RwLock<BTreeMap<String, String>>,
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
        let values = path
            .as_deref()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|raw| serde_json::from_str::<SettingsFile>(&raw).ok())
            .map(|file| file.values)
            .unwrap_or_default();
        Self {
            path,
            values: RwLock::new(values),
        }
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
        if let Some(path) = &self.path {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|error| format!("create directory: {error}"))?;
            }
            let staged = path.with_extension("json.tmp");
            let body = serde_json::to_string_pretty(&SettingsFile {
                values: next.clone(),
            })
            .map_err(|error| format!("encode settings: {error}"))?;
            fs::write(&staged, body).map_err(|error| format!("write settings: {error}"))?;
            fs::rename(&staged, path).map_err(|error| format!("replace settings: {error}"))?;
        }
        *guard = next;
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
        ] {
            assert!(
                !restart.contains(&key),
                "{key} is read per request and must not claim a restart"
            );
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
}
