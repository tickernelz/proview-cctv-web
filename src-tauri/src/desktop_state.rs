use std::io;
use std::path::Path;

use proview_server::desktop::DesktopAlarm;
use serde::{Deserialize, Serialize};

const KEEP: usize = 500;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopState {
    #[serde(default)]
    pub seen: Vec<String>,
    #[serde(default)]
    pub last_seen: i64,
    #[serde(default)]
    pub baseline_done: bool,
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

    pub fn absorb(&mut self, entries: &[DesktopAlarm]) -> Vec<DesktopAlarm> {
        let notify = self.baseline_done;
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
        self.baseline_done = true;
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
    fn the_first_pass_records_and_the_next_new_alarm_posts_once() {
        let mut state = DesktopState::default();
        let baseline = vec![alarm("a1", 10), alarm("a2", 11)];
        assert!(
            state.absorb(&baseline).is_empty(),
            "the first pass only marks what is already there"
        );
        assert_eq!(state.last_seen, 11);
        let fresh = state.absorb(&[alarm("a2", 11), alarm("a3", 12)]);
        assert_eq!(fresh.len(), 1, "only what arrived after the baseline");
        assert_eq!(fresh[0].id, "a3");
        assert!(
            state.absorb(&[alarm("a3", 12)]).is_empty(),
            "the same id does not post twice"
        );
    }

    #[test]
    fn an_empty_first_pass_still_ends_the_baseline() {
        let mut state = DesktopState::default();
        assert!(state.absorb(&[]).is_empty());
        let fresh = state.absorb(&[alarm("a1", 10)]);
        assert_eq!(
            fresh.len(),
            1,
            "an install with no alarms still announces the first one"
        );
    }

    #[test]
    fn a_restart_with_a_saved_state_does_not_replay_old_alarms() {
        let path = temp("restart");
        let mut state = DesktopState::default();
        state.absorb(&[alarm("a1", 10)]);
        state.save(&path).expect("save");
        let mut reloaded = DesktopState::load(&path);
        let replay = reloaded.absorb(&[alarm("a1", 10), alarm("a2", 12)]);
        assert_eq!(
            replay.len(),
            1,
            "only the alarm that arrived after the restart"
        );
        assert_eq!(replay[0].id, "a2");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_seen_list_stays_bounded() {
        let mut state = DesktopState::default();
        let many: Vec<DesktopAlarm> = (0..KEEP + 40)
            .map(|index| alarm(&format!("a{index}"), index as i64))
            .collect();
        state.absorb(&many);
        assert_eq!(state.seen.len(), KEEP);
        assert!(
            !state.seen.contains(&"a0".to_string()),
            "the oldest ids fall off"
        );
        assert!(state.seen.contains(&format!("a{}", KEEP + 39)));
    }
}
