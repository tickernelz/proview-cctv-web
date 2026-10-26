use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use proview_core::config::Config;
use proview_core::provider::{self, Mode, ProviderSource, Resolution};
use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};

pub const MODE_FILE: &str = "mode.json";
const SESSION_COOKIE: &str = "pvw_session";

#[derive(Default)]
struct Choices {
    global: Option<Mode>,
    sessions: HashMap<String, Mode>,
}

pub struct ModeStore {
    default: Mode,
    persist: Option<PathBuf>,
    choices: Mutex<Choices>,
}

impl ModeStore {
    pub fn new(default: Mode, persist: Option<PathBuf>) -> Self {
        let stored = persist.as_deref().and_then(read_mode);
        Self {
            default: stored.unwrap_or(default),
            persist,
            choices: Mutex::new(Choices::default()),
        }
    }

    pub fn from_env(data_dir: Option<&Path>) -> Self {
        let dir = data_dir.map(Path::to_path_buf).or_else(|| {
            std::env::var("PROVIEW_DATA_DIR")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        });
        let env_mode = std::env::var("PROVIEW_MODE")
            .ok()
            .as_deref()
            .and_then(Mode::parse)
            .unwrap_or_default();
        Self::new(env_mode, dir.map(|dir| dir.join(MODE_FILE)))
    }

    pub fn default_mode(&self) -> Mode {
        self.default
    }

    pub fn mode_for(&self, session: Option<&str>) -> Mode {
        let choices = self.choices.lock().expect("mode store poisoned");
        let chosen = session.and_then(|id| choices.sessions.get(id).copied());
        chosen.or(choices.global).unwrap_or(self.default)
    }

    pub fn choose(&self, session: Option<&str>, mode: Mode) -> Result<(), String> {
        if let Some(path) = &self.persist {
            write_mode(path, mode).map_err(|e| format!("could not persist mode: {e}"))?;
            let mut choices = self.choices.lock().expect("mode store poisoned");
            choices.global = Some(mode);
            return Ok(());
        }
        let Some(id) = session else {
            return Err("a session is required to choose a mode".to_string());
        };
        let mut choices = self.choices.lock().expect("mode store poisoned");
        choices.sessions.insert(id.to_string(), mode);
        Ok(())
    }

    pub fn forget(&self, session: &str) {
        let mut choices = self.choices.lock().expect("mode store poisoned");
        choices.sessions.remove(session);
    }

    pub fn persisted(&self) -> Option<&Path> {
        self.persist.as_deref()
    }
}

fn read_mode(path: &Path) -> Option<Mode> {
    let raw = fs::read_to_string(path).ok()?;
    let stored: StoredMode = serde_json::from_str(&raw).ok()?;
    Mode::parse(&stored.mode)
}

fn write_mode(path: &Path, mode: Mode) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let staged = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(&StoredMode {
        mode: mode.as_str().to_string(),
    })
    .map_err(std::io::Error::other)?;
    fs::write(&staged, body)?;
    fs::rename(staged, path)
}

#[derive(Serialize, Deserialize)]
struct StoredMode {
    mode: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModeReport {
    pub mode: Mode,
    pub source: ProviderSource,
    pub notice: Option<provider::Notice>,
}

impl From<Resolution> for ModeReport {
    fn from(resolution: Resolution) -> Self {
        Self {
            mode: resolution.mode,
            source: resolution.source,
            notice: resolution.notice,
        }
    }
}

#[derive(Deserialize)]
pub struct ModeRequest {
    pub mode: Mode,
}

#[derive(Serialize)]
struct ModeError {
    ok: bool,
    message: String,
}

type ModeFailure = (Status, Json<ModeError>);

fn refuse(status: Status, message: impl Into<String>) -> ModeFailure {
    (
        status,
        Json(ModeError {
            ok: false,
            message: message.into(),
        }),
    )
}

pub fn routes() -> Vec<rocket::route::Route> {
    routes![report_mode, choose_mode]
}

fn live_session(store: &Store, cookies: &CookieJar<'_>) -> Option<String> {
    let id = cookies.get_private(SESSION_COOKIE)?.value().to_string();
    store.get(&id).map(|_| id)
}

#[get("/config/mode")]
fn report_mode(
    cfg: &State<Config>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Json<ModeReport> {
    let session = live_session(store, cookies);
    let mode = modes.mode_for(session.as_deref());
    Json(provider::plan(cfg, mode).into())
}

#[post("/config/mode", data = "<body>")]
fn choose_mode(
    body: Json<ModeRequest>,
    cfg: &State<Config>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<ModeReport>, ModeFailure> {
    let session = live_session(store, cookies)
        .ok_or_else(|| refuse(Status::Unauthorized, "not signed in"))?;
    modes
        .choose(Some(&session), body.mode)
        .map_err(|message| refuse(Status::InternalServerError, message))?;
    Ok(Json(provider::plan(cfg, body.mode).into()))
}

#[cfg(test)]
mod tests {
    use super::ModeStore;
    use proview_core::provider::Mode;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("proview-mode-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_choice_is_persisted_for_the_next_start() {
        let dir = scratch("persist");
        let path = dir.join(super::MODE_FILE);

        let store = ModeStore::new(Mode::Cloud, Some(path.clone()));
        assert_eq!(store.mode_for(None), Mode::Cloud);
        store
            .choose(None, Mode::Local)
            .expect("local is persistable");
        assert_eq!(store.mode_for(None), Mode::Local);

        let reopened = ModeStore::new(Mode::Cloud, Some(path));
        assert_eq!(reopened.mode_for(None), Mode::Local);
        assert_eq!(reopened.default_mode(), Mode::Local);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_persisted_choice_wins_over_the_bootstrap_default() {
        let dir = scratch("global");
        let path = dir.join(super::MODE_FILE);
        let store = ModeStore::new(Mode::Cloud, Some(path));
        store.choose(Some("alpha"), Mode::Local).expect("persisted");
        assert_eq!(store.mode_for(Some("alpha")), Mode::Local);
        assert_eq!(store.mode_for(Some("beta")), Mode::Local);
        assert_eq!(store.mode_for(None), Mode::Local);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_server_mode_is_kept_per_session() {
        let store = ModeStore::new(Mode::Cloud, None);
        store
            .choose(Some("alpha"), Mode::Local)
            .expect("alpha may switch");
        assert_eq!(store.mode_for(Some("alpha")), Mode::Local);
        assert_eq!(store.mode_for(Some("beta")), Mode::Cloud);
        assert_eq!(store.mode_for(None), Mode::Cloud);
        store.forget("alpha");
        assert_eq!(store.mode_for(Some("alpha")), Mode::Cloud);
        assert!(store.choose(None, Mode::Local).is_err());
    }
}
