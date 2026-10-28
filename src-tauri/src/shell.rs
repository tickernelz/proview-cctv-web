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
