use std::io;
use std::path::Path;

use proview_server::desktop::DesktopAlarm;
use serde::{Deserialize, Serialize};

const KEEP: usize = 500;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopState {
    #[serde(default)]
    pub seen: Vec<String>,
    #[serde(default)]
    pub last_seen: i64,
    #[serde(default)]
    pub tray_hint: bool,
}

impl DesktopState {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let staged = path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        std::fs::write(&staged, body)?;
        std::fs::rename(&staged, path)
    }

    pub fn absorb(&mut self, entries: &[DesktopAlarm], notify: bool) -> Vec<DesktopAlarm> {
        let mut fresh = Vec::new();
        for entry in entries {
            if self.seen.iter().any(|seen| seen == &entry.id) {
                continue;
            }
            self.seen.push(entry.id.clone());
            if self.seen.len() > KEEP {
                let drop = self.seen.len() - KEEP;
                self.seen.drain(..drop);
            }
            self.last_seen = self.last_seen.max(entry.utc_time);
            if notify {
                fresh.push(entry.clone());
            }
        }
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alarm(id: &str, utc_time: i64) -> DesktopAlarm {
        DesktopAlarm {
            id: id.to_string(),
            serial: "DEV0001".to_string(),
            device: "Gate".to_string(),
            event: "Motion".to_string(),
            time: "2026-10-28 01:02:03".to_string(),
            utc_time,
        }
    }

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("proview-state-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir.join("desktop-state.json")
    }

    #[test]
    fn the_first_sight_of_an_alarm_notifies_once() {
        let mut state = DesktopState::default();
        let batch = vec![alarm("a1", 10), alarm("a2", 11)];
        assert_eq!(state.absorb(&batch, true).len(), 2);
        assert!(
            state.absorb(&batch, true).is_empty(),
            "same ids, no second notification"
        );
        assert_eq!(state.last_seen, 11);
    }

    #[test]
    fn a_restart_with_a_saved_state_does_not_replay_old_alarms() {
        let path = temp("restart");
        let mut state = DesktopState::default();
        state.absorb(&[alarm("a1", 10)], true);
        state.save(&path).expect("save");
        let mut reloaded = DesktopState::load(&path);
        let replay = reloaded.absorb(&[alarm("a1", 10), alarm("a2", 12)], true);
        assert_eq!(
            replay.len(),
            1,
            "only the alarm that arrived after the restart"
        );
        assert_eq!(replay[0].id, "a2");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_baseline_records_without_notifying() {
        let mut state = DesktopState::default();
        let quiet = state.absorb(&[alarm("a1", 10), alarm("a2", 12)], false);
        assert!(
            quiet.is_empty(),
            "the first pass only marks what is already there"
        );
        assert_eq!(state.last_seen, 12);
        let later = state.absorb(&[alarm("a3", 13)], true);
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].id, "a3");
    }

    #[test]
    fn the_seen_list_stays_bounded() {
        let mut state = DesktopState::default();
        let many: Vec<DesktopAlarm> = (0..KEEP + 40)
            .map(|index| alarm(&format!("a{index}"), index as i64))
            .collect();
        state.absorb(&many, false);
        assert_eq!(state.seen.len(), KEEP);
        assert!(
            !state.seen.contains(&"a0".to_string()),
            "the oldest ids fall off"
        );
        assert!(state.seen.contains(&format!("a{}", KEEP + 39)));
    }
}
