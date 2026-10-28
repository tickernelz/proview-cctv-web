use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use proview_server::settings::SettingsStore;

use crate::desktop_settings::DesktopSettings;
use crate::desktop_state::DesktopState;

pub struct DesktopShell {
    pub settings: SettingsStore,
    pub state: Arc<Mutex<DesktopState>>,
    pub state_path: Option<PathBuf>,
    tray_ready: AtomicBool,
}

impl DesktopShell {
    pub fn new(settings: SettingsStore, state_path: Option<PathBuf>) -> Self {
        let state = state_path
            .as_deref()
            .map(DesktopState::load)
            .unwrap_or_default();
        Self {
            settings,
            state: Arc::new(Mutex::new(state)),
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
