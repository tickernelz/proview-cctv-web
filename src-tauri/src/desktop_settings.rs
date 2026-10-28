use proview_server::settings::{bounds_of, default_of, SettingsStore};

pub const NOTIFY: &str = "PROVIEW_DESKTOP_ALARM_NOTIFY";
pub const INTERVAL: &str = "PROVIEW_DESKTOP_ALARM_INTERVAL";
pub const CLOSE_TO_TRAY: &str = "PROVIEW_DESKTOP_CLOSE_TO_TRAY";
pub const MINIMIZE_TO_TRAY: &str = "PROVIEW_DESKTOP_MINIMIZE_TO_TRAY";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopSettings {
    pub alarm_notify: bool,
    pub interval_secs: u64,
    pub close_to_tray: bool,
    pub minimize_to_tray: bool,
}

impl DesktopSettings {
    pub fn read(store: &SettingsStore) -> Self {
        Self {
            alarm_notify: toggle(store, NOTIFY),
            interval_secs: interval(store),
            close_to_tray: toggle(store, CLOSE_TO_TRAY),
            minimize_to_tray: toggle(store, MINIMIZE_TO_TRAY),
        }
    }
}

fn value_of(store: &SettingsStore, key: &str) -> Option<String> {
    store
        .value(key)
        .or_else(|| default_of(key).map(str::to_string))
}

fn toggle(store: &SettingsStore, key: &str) -> bool {
    value_of(store, key).as_deref() == Some("on")
}

fn interval(store: &SettingsStore) -> u64 {
    let stored = value_of(store, INTERVAL).and_then(|value| value.parse::<u64>().ok());
    let Some(seconds) = stored else {
        return default_of(INTERVAL)
            .and_then(|value| value.parse().ok())
            .unwrap_or(60);
    };
    match bounds_of(INTERVAL) {
        Some((low, high)) => seconds.clamp(low, high),
        None => seconds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn store(tag: &str) -> (SettingsStore, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("proview-desktop-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        (SettingsStore::load(Some(&dir)), dir)
    }

    #[test]
    fn an_untouched_profile_runs_the_documented_defaults() {
        let (store, dir) = store("defaults");
        let settings = DesktopSettings::read(&store);
        assert!(!settings.alarm_notify, "notifications are opt-in");
        assert_eq!(settings.interval_secs, 60);
        assert!(settings.close_to_tray, "closing the window keeps the app");
        assert!(!settings.minimize_to_tray);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_saved_profile_is_read_back() {
        let (store, dir) = store("saved");
        let mut values = BTreeMap::new();
        values.insert(NOTIFY.to_string(), "on".to_string());
        values.insert(INTERVAL.to_string(), "120".to_string());
        values.insert(CLOSE_TO_TRAY.to_string(), "off".to_string());
        values.insert(MINIMIZE_TO_TRAY.to_string(), "on".to_string());
        store.update(values).expect("save");
        let settings = DesktopSettings::read(&store);
        assert!(settings.alarm_notify);
        assert_eq!(settings.interval_secs, 120);
        assert!(!settings.close_to_tray);
        assert!(settings.minimize_to_tray);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_out_of_range_interval_falls_back_into_the_bounds() {
        let dir =
            std::env::temp_dir().join(format!("proview-desktop-{}-bounds", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(
            dir.join("settings.json"),
            "{\"values\":{\"PROVIEW_DESKTOP_ALARM_INTERVAL\":\"3\"}}",
        )
        .expect("settings file");
        let store = SettingsStore::load(Some(&dir));
        assert_eq!(
            DesktopSettings::read(&store).interval_secs,
            15,
            "a hand-edited file cannot run the check faster than the bounds allow"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
