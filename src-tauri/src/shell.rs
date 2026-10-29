use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use proview_core::session::Store;
use proview_server::settings::SettingsStore;

use crate::desktop_settings::DesktopSettings;
use crate::desktop_state::{now_seconds, DesktopState, Run};

pub struct DesktopShell {
    pub settings: SettingsStore,
    pub state: Arc<Mutex<DesktopState>>,
    pub sessions: Store,
    pub alarm: Mutex<Run>,
    pub state_path: Option<PathBuf>,
    tray_ready: AtomicBool,
    status: Mutex<String>,
    ended_at: Mutex<Option<String>>,
}

impl DesktopShell {
    pub fn new(settings: SettingsStore, state_path: Option<PathBuf>, sessions: Store) -> Self {
        let state = state_path
            .as_deref()
            .map(DesktopState::load)
            .unwrap_or_default();
        Self {
            settings,
            state: Arc::new(Mutex::new(state)),
            sessions,
            alarm: Mutex::new(Run::new(now_seconds())),
            state_path,
            tray_ready: AtomicBool::new(false),
            status: Mutex::new(String::new()),
            ended_at: Mutex::new(None),
        }
    }

    pub fn current(&self) -> DesktopSettings {
        DesktopSettings::read(&self.settings)
    }

    pub fn tray_ready(&self) -> bool {
        self.tray_ready.load(Ordering::Relaxed)
    }

    pub fn set_tray_ready(&self, ready: bool) {
        self.tray_ready.store(ready, Ordering::Relaxed);
    }

    pub fn restart_alarm_run(&self, started: i64) {
        if let Ok(mut alarm) = self.alarm.lock() {
            *alarm = Run::new(started);
        }
    }

    pub fn take_tray_pause_notice(&self) -> bool {
        let signed_in = self.sessions.latest().is_some();
        match self.alarm.lock() {
            Ok(mut alarm) => alarm.tray_pause_notice(signed_in),
            Err(_) => false,
        }
    }

    pub fn tray_status(&self, notify_on: bool) -> String {
        let live = crate::status::clock_in(self.sessions.latest_remaining());
        if let Some(label) = live.as_ref() {
            if let Ok(mut ended) = self.ended_at.lock() {
                *ended = Some(label.clone());
            }
        }
        let ended = self.ended_at.lock().ok().and_then(|ended| ended.clone());
        crate::status::label(notify_on, live.as_deref(), ended.as_deref())
    }

    pub fn refresh_status(&self, app: &tauri::AppHandle, notify_on: bool) {
        let text = self.tray_status(notify_on);
        let Ok(mut known) = self.status.lock() else {
            return;
        };
        if *known == text {
            return;
        }
        *known = text.clone();
        crate::tray::set_status(app, &text);
    }

    pub fn save(&self) {
        let Some(path) = self.state_path.as_deref() else {
            return;
        };
        let snapshot = match self.state.lock() {
            Ok(state) => (*state).clone(),
            Err(_) => return,
        };
        if let Err(error) = snapshot.save(path) {
            tracing::warn!(%error, "could not save the desktop state");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proview_core::provider::VendorSession;

    fn shell(tag: &str) -> (DesktopShell, PathBuf) {
        let dir = std::env::temp_dir().join(format!("proview-shell-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let shell = DesktopShell::new(SettingsStore::load(Some(&dir)), None, Store::new());
        (shell, dir)
    }

    #[test]
    fn the_hide_to_tray_pause_starts_a_fresh_run_when_the_switch_flips() {
        let (shell, dir) = shell("pause-reset");
        assert!(
            shell.take_tray_pause_notice(),
            "no session and no successful check, so the pause is the truth"
        );
        assert!(
            !shell.take_tray_pause_notice(),
            "the second hide in the same run stays quiet"
        );
        shell.restart_alarm_run(0);
        assert!(shell.take_tray_pause_notice(), "a new run may speak again");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_tray_line_names_when_the_session_ends_and_keeps_that_hour_once_it_has() {
        let (shell, dir) = shell("status");
        shell.sessions.put(
            "test".to_string(),
            VendorSession {
                account: "gate".to_string(),
                token: None,
            },
            std::time::Duration::from_secs(600),
        );
        let live = shell.tray_status(true);
        let hour = live
            .rsplit("session ends ")
            .next()
            .expect("the line names an hour")
            .to_string();
        assert_eq!(
            live,
            format!("Alarm notifications on · session ends {hour}")
        );
        shell.sessions.remove("test");
        assert_eq!(
            shell.tray_status(true),
            format!("Alarm notifications paused · session ended {hour}"),
            "the line keeps the hour the last session ended at"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_switched_off_feature_says_so_in_the_tray() {
        let (shell, dir) = shell("status-off");
        assert_eq!(shell.tray_status(false), "Alarm notifications off");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_live_session_leaves_the_tray_quiet() {
        let (shell, dir) = shell("pause-signed-in");
        shell.sessions.put(
            "test".to_string(),
            VendorSession {
                account: "gate".to_string(),
                token: None,
            },
            std::time::Duration::from_secs(600),
        );
        assert!(
            !shell.take_tray_pause_notice(),
            "the store holds a live session, so nothing is paused yet"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
