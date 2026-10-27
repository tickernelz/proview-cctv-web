use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

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
        true,
    ),
    field(
        "PROVIEW_CAPS_BASE",
        "Caps base",
        "Cloud",
        Kind::Text,
        &[],
        "Host that answers device capabilities by serial.",
        true,
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
        false,
    ),
    field(
        "PROVIEW_DEVICE_RTSP",
        "Device RTSP",
        "Local network",
        Kind::Text,
        &[],
        "RTSP host and port, usually 554.",
        false,
    ),
    field(
        "PROVIEW_DEVICE_ONVIF",
        "Device ONVIF",
        "Local network",
        Kind::Text,
        &[],
        "ONVIF base address for movement and presets.",
        false,
    ),
    field(
        "PROVIEW_DEVICE_USER",
        "Device user",
        "Local network",
        Kind::Text,
        &[],
        "Camera user for LAN access.",
        false,
    ),
    field(
        "PROVIEW_DEVICE_PASS",
        "Device password",
        "Local network",
        Kind::Secret,
        &[],
        "Camera password for LAN access; stored in the settings file.",
        false,
    ),
    field(
        "PROVIEW_SESSION_TTL",
        "Session seconds",
        "Server",
        Kind::Number,
        &[],
        "Browser session lifetime.",
        false,
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
        false,
    ),
    field(
        "PROVIEW_PROFILE",
        "Keychain profile",
        "Advanced",
        Kind::Text,
        &[],
        "Profile name the desktop uses for stored device credentials.",
        false,
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

#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: Option<PathBuf>,
    values: BTreeMap<String, String>,
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
        Self { path, values }
    }

    pub fn apply_env(&self) {
        for (key, value) in &self.values {
            if value.trim().is_empty() || field_for(key).is_none() {
                continue;
            }
            if env::var_os(key).is_none() {
                env::set_var(key, value);
            }
        }
    }

    pub fn view(&self) -> SettingsView {
        let fields = FIELDS
            .iter()
            .map(|field| {
                let secret = field.kind == Kind::Secret;
                let current = env::var(field.key)
                    .ok()
                    .filter(|value| !value.trim().is_empty());
                let stored = self
                    .values
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

    pub fn update(&mut self, incoming: BTreeMap<String, String>) -> Result<SettingsView, String> {
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
        let mut next = self.values.clone();
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
        self.values = next;
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
    let mut guard = store.inner().clone();
    guard
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
        let (mut store, dir) = temp_store("unknown");
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
        let (mut store, dir) = temp_store("secrets");
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
    fn clears_a_value() {
        let (mut store, dir) = temp_store("clear");
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
