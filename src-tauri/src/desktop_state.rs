use std::io;
use std::path::Path;

use proview_server::desktop_alarms::DesktopAlarm;
use serde::{Deserialize, Serialize};

pub const OVERLAP_SECS: i64 = 120;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeenAlarm {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub utc_time: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Silent,
    Notify { cutoff: i64 },
}

#[derive(Debug, Default)]
pub struct Batch {
    pub fresh: Vec<DesktopAlarm>,
    pub skipped: usize,
    pub changed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopState {
    #[serde(default)]
    pub seen: Vec<SeenAlarm>,
    #[serde(default)]
    pub last_seen: i64,
    #[serde(default)]
    pub baseline_done: bool,
    #[serde(default)]
    pub tray_hint: bool,
    #[serde(default)]
    pub account: String,
    #[serde(default)]
    pub port: u16,
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

    pub fn switch_account(&mut self, account: &str) -> bool {
        if self.account == account {
            return false;
        }
        self.account = account.to_string();
        self.seen.clear();
        self.last_seen = 0;
        true
    }

    pub fn absorb(
        &mut self,
        entries: &[DesktopAlarm],
        policy: Policy,
        floor: i64,
        now: i64,
    ) -> Batch {
        let before = self.seen.len();
        self.seen.retain(|seen| seen.utc_time >= floor);
        let mut batch = Batch {
            changed: self.seen.len() != before,
            ..Batch::default()
        };
        let ceiling = now + OVERLAP_SECS;
        for entry in entries {
            if entry.utc_time < floor {
                continue;
            }
            let key = entry.key();
            if self.seen.iter().any(|seen| seen.key == key) {
                continue;
            }
            self.seen.push(SeenAlarm {
                key,
                utc_time: entry.utc_time,
            });
            self.last_seen = self.last_seen.max(entry.utc_time.min(ceiling));
            batch.changed = true;
            match policy {
                Policy::Silent => batch.skipped += 1,
                Policy::Notify { cutoff } => {
                    if entry.utc_time < cutoff {
                        batch.skipped += 1;
                    } else {
                        batch.fresh.push(entry.clone());
                    }
                }
            }
        }
        batch.changed |= !std::mem::replace(&mut self.baseline_done, true);
        batch
    }
}

pub fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

pub struct Run {
    session_start: i64,
    first_check: bool,
    last_ok: Option<i64>,
    failed_checks: usize,
    pause_posted: bool,
    tray_pause_posted: bool,
}

impl Run {
    pub fn new(session_start: i64) -> Self {
        Self {
            session_start,
            first_check: true,
            last_ok: None,
            failed_checks: 0,
            pause_posted: false,
            tray_pause_posted: false,
        }
    }

    pub fn policy(&self, state: &DesktopState, interval: u64, now: i64) -> Policy {
        if !state.baseline_done {
            return Policy::Silent;
        }
        if self.first_check {
            return Policy::Notify {
                cutoff: self.session_start - interval as i64 - OVERLAP_SECS,
            };
        }
        if self.failed_checks >= 2 {
            return Policy::Notify { cutoff: now };
        }
        Policy::Notify { cutoff: i64::MIN }
    }

    pub fn failed(&mut self, session_lost: bool) -> bool {
        self.failed_checks += 1;
        let announced = session_lost && self.last_ok.is_some() && !self.pause_posted;
        if announced {
            self.pause_posted = true;
        }
        announced
    }

    pub fn finished(&mut self, ok: bool, now: i64) {
        if ok {
            self.first_check = false;
            self.failed_checks = 0;
            self.pause_posted = false;
            self.last_ok = Some(now);
        }
    }

    pub fn tray_pause_notice(&mut self, signed_in: bool) -> bool {
        if signed_in || self.last_ok.is_some() || self.tray_pause_posted {
            return false;
        }
        self.tray_pause_posted = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proview_server::desktop_alarms::AlarmReadError;

    fn alarm(serial: &str, channel: &str, id: &str, utc_time: i64) -> DesktopAlarm {
        DesktopAlarm {
            id: id.to_string(),
            serial: serial.to_string(),
            channel: channel.to_string(),
            device: "Gate".to_string(),
            event: "Motion".to_string(),
            time: "2026-10-28 01:02:03".to_string(),
            utc_time,
        }
    }

    fn gate(id: &str, utc_time: i64) -> DesktopAlarm {
        alarm("DEV0001", "0", id, utc_time)
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
        let baseline = vec![gate("a1", 10), gate("a2", 11)];
        let quiet = state.absorb(&baseline, Policy::Silent, 0, 100);
        assert!(quiet.fresh.is_empty(), "the first pass only records");
        assert_eq!(quiet.skipped, 2);
        assert_eq!(state.last_seen, 11);
        let fresh = state.absorb(
            &[gate("a2", 11), gate("a3", 12)],
            Policy::Notify { cutoff: i64::MIN },
            0,
            100,
        );
        assert_eq!(fresh.fresh.len(), 1, "only what arrived after the baseline");
        assert_eq!(fresh.fresh[0].id, "a3");
        let again = state.absorb(
            &[gate("a3", 12)],
            Policy::Notify { cutoff: i64::MIN },
            0,
            100,
        );
        assert!(again.fresh.is_empty(), "the same key does not post twice");
    }

    #[test]
    fn an_empty_first_pass_still_ends_the_baseline() {
        let mut state = DesktopState::default();
        assert!(!state.baseline_done);
        assert!(state.absorb(&[], Policy::Silent, 0, 100).fresh.is_empty());
        assert!(
            state.baseline_done,
            "an empty first pass still ends the baseline"
        );
        let fresh = state.absorb(
            &[gate("a1", 10)],
            Policy::Notify { cutoff: i64::MIN },
            0,
            100,
        );
        assert_eq!(
            fresh.fresh.len(),
            1,
            "an install with no alarms still announces the first one"
        );
    }

    #[test]
    fn the_page_port_survives_a_restart_and_an_older_file_reads_as_none() {
        let path = temp("port");
        let state = DesktopState {
            port: 51097,
            ..DesktopState::default()
        };
        state.save(&path).expect("save");
        assert_eq!(DesktopState::load(&path).port, 51097);
        std::fs::write(&path, r#"{"seen":[],"lastSeen":5,"baselineDone":true}"#)
            .expect("older file");
        let older = DesktopState::load(&path);
        assert_eq!(older.port, 0, "no saved port means a fresh one");
        assert!(older.baseline_done, "the rest of an older file still loads");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_baseline_flag_survives_a_restart() {
        let path = temp("baseline");
        let mut state = DesktopState::default();
        state.absorb(&[], Policy::Silent, 0, 100);
        state.save(&path).expect("save");
        let reloaded = DesktopState::load(&path);
        assert!(reloaded.baseline_done, "the next run is not a baseline run");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_alarm_below_the_floor_is_ignored_instead_of_posting_again() {
        let mut state = DesktopState::default();
        state.absorb(&[], Policy::Silent, 0, 100_000);
        let old = gate("old18h", 90_000);
        let floor = 95_000;
        let first = state.absorb(
            std::slice::from_ref(&old),
            Policy::Notify { cutoff: i64::MIN },
            floor,
            100_000,
        );
        assert!(
            first.fresh.is_empty(),
            "the centre may keep returning it; the floor decides"
        );
        let second = state.absorb(&[old], Policy::Notify { cutoff: i64::MIN }, floor, 100_000);
        assert!(second.fresh.is_empty());
        assert!(!second.changed, "nothing was recorded either");
    }

    #[test]
    fn a_restart_with_a_saved_state_does_not_replay_old_alarms() {
        let path = temp("restart");
        let mut state = DesktopState::default();
        state.absorb(&[gate("a1", 10)], Policy::Silent, 0, 100);
        state.save(&path).expect("save");
        let mut reloaded = DesktopState::load(&path);
        let replay = reloaded.absorb(
            &[gate("a1", 10), gate("a2", 12)],
            Policy::Notify { cutoff: i64::MIN },
            0,
            100,
        );
        assert_eq!(
            replay.fresh.len(),
            1,
            "only the alarm that arrived after the restart"
        );
        assert_eq!(replay.fresh[0].id, "a2");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn thirty_alarms_from_before_this_run_produce_one_summary_and_no_toast() {
        let path = temp("downtime");
        let mut state = DesktopState::default();
        state.absorb(&[gate("first", 6_000_000)], Policy::Silent, 0, 6_000_000);
        state.save(&path).expect("save");
        let mut reloaded = DesktopState::load(&path);
        let run_start = 6_003_600;
        let cutoff = run_start - 60 - OVERLAP_SECS;
        let missed: Vec<DesktopAlarm> = (0..30)
            .map(|index| gate(&format!("m{index}"), 6_000_000 + index * 60))
            .collect();
        let batch = reloaded.absorb(&missed, Policy::Notify { cutoff }, 0, run_start);
        assert!(
            batch.fresh.is_empty(),
            "a run that starts after a downtime must not toast every missed alarm"
        );
        assert_eq!(batch.skipped, 30, "the summary counts them instead");
        let during_run = reloaded.absorb(
            &[gate("live", run_start + 5)],
            Policy::Notify { cutoff },
            0,
            run_start,
        );
        assert_eq!(
            during_run.fresh.len(),
            1,
            "what happens during the run toasts"
        );
    }

    #[test]
    fn two_devices_that_share_an_alarm_id_both_post() {
        let mut state = DesktopState::default();
        state.absorb(&[], Policy::Silent, 0, 100);
        let batch = state.absorb(
            &[
                alarm("DEV0001", "0", "261027171005", 10),
                alarm("DEV0002", "0", "261027171005", 10),
            ],
            Policy::Notify { cutoff: i64::MIN },
            0,
            100,
        );
        assert_eq!(batch.fresh.len(), 2, "the key carries the device serial");
    }

    #[test]
    fn a_window_full_of_alarms_does_not_let_an_id_come_back() {
        let mut state = DesktopState::default();
        state.absorb(&[], Policy::Silent, 0, 100_000);
        let window: Vec<DesktopAlarm> = (0..600)
            .map(|index| gate(&format!("a{index}"), 90_000 + index))
            .collect();
        let first = state.absorb(&window, Policy::Notify { cutoff: i64::MIN }, 0, 100_000);
        assert_eq!(
            first.fresh.len(),
            600,
            "every alarm in the window posts once"
        );
        let second = state.absorb(&window, Policy::Notify { cutoff: i64::MIN }, 0, 100_000);
        assert!(
            second.fresh.is_empty(),
            "nothing comes back however many alarms the window holds"
        );
        assert_eq!(state.seen.len(), 600);
    }

    #[test]
    fn the_watermark_never_moves_past_the_check_time() {
        let mut state = DesktopState::default();
        let now = 1_000;
        state.absorb(
            &[gate("future", now + 100_000)],
            Policy::Notify { cutoff: i64::MIN },
            0,
            now,
        );
        assert_eq!(
            state.last_seen,
            now + OVERLAP_SECS,
            "one device with a clock far ahead cannot push the window past every other device"
        );
    }

    #[test]
    fn a_seen_alarm_older_than_the_floor_is_forgotten() {
        let mut state = DesktopState::default();
        state.absorb(&[gate("old", 10)], Policy::Silent, 0, 100);
        assert_eq!(state.seen.len(), 1);
        let batch = state.absorb(&[], Policy::Silent, 50, 100);
        assert!(batch.changed, "dropping an entry is a change worth saving");
        assert!(state.seen.is_empty(), "the floor is the window start");
    }

    #[test]
    fn an_empty_first_pass_reports_the_baseline_as_a_change() {
        let mut state = DesktopState::default();
        let batch = state.absorb(&[], Policy::Silent, 0, 10_000);
        assert!(state.baseline_done);
        assert!(
            batch.changed,
            "the watcher only saves on a change, so the baseline flag must count as one"
        );
    }

    #[test]
    fn another_account_starts_from_a_clean_slate() {
        let mut state = DesktopState::default();
        assert!(
            state.switch_account("owner@example.test"),
            "the first sign-in names the window"
        );
        state.absorb(&[gate("a", 10)], Policy::Silent, 0, 100);
        assert_eq!(state.seen.len(), 1);
        assert!(
            !state.switch_account("owner@example.test"),
            "the same account keeps its window"
        );
        assert!(
            state.switch_account("other@example.test"),
            "a different account drops the window"
        );
        assert!(
            state.seen.is_empty(),
            "the old account's alarms cannot silence the new one"
        );
        assert_eq!(
            state.last_seen, 0,
            "the window restarts from the new sign-in"
        );
        assert_eq!(state.account, "other@example.test");
    }

    fn run_alarm(id: &str, utc_time: i64) -> DesktopAlarm {
        DesktopAlarm {
            id: id.to_string(),
            serial: "DEV0001".to_string(),
            channel: "0".to_string(),
            device: "Gate".to_string(),
            event: "Motion".to_string(),
            time: "2026-10-28 01:02:03".to_string(),
            utc_time,
        }
    }

    fn awake_state() -> DesktopState {
        DesktopState {
            baseline_done: true,
            last_seen: 9_000,
            ..DesktopState::default()
        }
    }

    #[test]
    fn a_failed_first_check_does_not_arm_the_flood_gate() {
        let mut state = awake_state();
        let mut run = Run::new(10_000);
        run.finished(false, 10_000);
        let policy = run.policy(&state, 60, 10_000);
        assert_eq!(
            policy,
            Policy::Notify {
                cutoff: 10_000 - 60 - OVERLAP_SECS
            },
            "the check that succeeded is the one that hears the cutoff"
        );
        let missed: Vec<DesktopAlarm> = (0..30)
            .map(|index| run_alarm(&format!("m{index}"), 9_000 + index))
            .collect();
        let batch = state.absorb(&missed, policy, 0, 10_000);
        assert!(
            batch.fresh.is_empty(),
            "nothing toasts for the missed window"
        );
        assert_eq!(batch.skipped, 30, "one summary counts them instead");
        run.finished(true, 10_000);
        assert_eq!(
            run.policy(&state, 60, 10_030),
            Policy::Notify { cutoff: i64::MIN },
            "after a successful check the run is awake"
        );
    }

    #[test]
    fn an_empty_baseline_pass_lets_the_next_alarm_through() {
        let mut state = DesktopState::default();
        let mut run = Run::new(10_000);
        let policy = run.policy(&state, 60, 10_000);
        assert_eq!(policy, Policy::Silent, "a fresh install records quietly");
        let quiet = state.absorb(&[], policy, 0, 10_000);
        assert!(quiet.fresh.is_empty());
        assert!(quiet.changed, "the baseline flag itself is a state change");
        run.finished(true, 10_000);
        assert_eq!(
            run.policy(&state, 60, 10_060),
            Policy::Notify { cutoff: i64::MIN },
            "the empty baseline pass still ends the baseline"
        );
        let batch = state.absorb(
            &[run_alarm("first", 10_010)],
            Policy::Notify { cutoff: i64::MIN },
            0,
            10_060,
        );
        assert_eq!(batch.fresh.len(), 1, "the first real alarm is announced");
    }

    #[test]
    fn a_switch_flip_starts_a_run_with_the_gate_armed() {
        let state = awake_state();
        let run = Run::new(50_000);
        assert_eq!(
            run.policy(&state, 15, 50_000),
            Policy::Notify {
                cutoff: 50_000 - 15 - OVERLAP_SECS
            },
            "turning the switch on is a run start, not a wake-up"
        );
    }

    #[test]
    fn two_failed_checks_summarize_the_gap_instead_of_toasting_it() {
        let mut state = awake_state();
        let mut run = Run::new(1_000);
        run.finished(true, 1_000);
        assert!(run.failed(true), "the pause notice is posted once");
        let now = 1_060;
        assert_eq!(
            run.policy(&state, 15, now),
            Policy::Notify { cutoff: i64::MIN },
            "one missed check is not an outage"
        );
        run.failed(true);
        let policy = run.policy(&state, 15, now);
        assert_eq!(
            policy,
            Policy::Notify { cutoff: now },
            "two missed checks summarize what the recovering check finds"
        );
        let gap: Vec<DesktopAlarm> = (0..15)
            .map(|index| run_alarm(&format!("g{index}"), 1_010 + index))
            .collect();
        let batch = state.absorb(&gap, policy, 0, now);
        assert!(batch.fresh.is_empty(), "no gap alarm toasts on its own");
        assert_eq!(batch.skipped, 15, "one summary counts the whole gap");
        run.finished(true, now);
        assert_eq!(
            run.policy(&state, 15, now + 15),
            Policy::Notify { cutoff: i64::MIN },
            "the check after the recovery toasts normally again"
        );
    }

    #[test]
    fn a_single_failure_still_toasts_the_next_alarm() {
        let mut state = awake_state();
        let mut run = Run::new(1_000);
        run.finished(true, 1_000);
        assert!(run.failed(true));
        let policy = run.policy(&state, 15, 1_030);
        assert_eq!(
            policy,
            Policy::Notify { cutoff: i64::MIN },
            "one missed interval is not an outage"
        );
        let batch = state.absorb(&[run_alarm("fresh", 1_025)], policy, 0, 1_030);
        assert_eq!(batch.fresh.len(), 1, "a single failure keeps the toast");
        assert_eq!(batch.skipped, 0);
    }

    #[test]
    fn a_hide_before_any_success_announces_the_pause_once() {
        let mut run = Run::new(0);
        assert!(
            run.tray_pause_notice(false),
            "no session and no successful check, so the pause is the truth"
        );
        assert!(
            !run.tray_pause_notice(false),
            "a second hide in the same run stays quiet"
        );
        run.finished(true, 10);
        assert!(
            !run.tray_pause_notice(false),
            "a run whose check succeeded is not paused"
        );
        assert!(
            run.failed(true),
            "the watcher still owns the pause once the session returns"
        );
    }

    #[test]
    fn a_hide_with_a_live_session_stays_quiet_before_the_first_success() {
        let mut run = Run::new(0);
        assert!(
            !run.tray_pause_notice(true),
            "the page already signed in; the next check can only agree with it"
        );
        assert!(
            !run.failed(true),
            "nothing was interrupted before the first success"
        );
        assert!(
            run.tray_pause_notice(false),
            "once the session is gone the pause is the truth"
        );
    }

    #[test]
    fn a_pause_before_any_success_stays_quiet() {
        let mut run = Run::new(500);
        assert!(
            !run.failed(true),
            "the store is empty on every launch; until a check succeeds nothing was interrupted"
        );
        assert!(!run.failed(true), "still nothing to announce");
        run.finished(true, 515);
        assert!(
            run.failed(true),
            "once a check has succeeded the pause is real"
        );
    }

    #[test]
    fn the_pause_notice_is_posted_once_per_failed_span() {
        let mut run = Run::new(0);
        run.finished(true, 0);
        assert!(run.failed(true), "the first session failure announces");
        assert!(!run.failed(true), "the rest of the span stays quiet");
        assert!(!run.failed(false), "another failure keeps the span");
        run.finished(true, 40);
        assert!(run.failed(true), "a new span announces again");
    }

    #[test]
    fn a_failure_that_is_not_the_session_posts_no_pause() {
        let mut run = Run::new(0);
        run.finished(true, 0);
        assert!(!run.failed(false), "a cloud hiccup is not a pause");
        assert!(run.failed(true), "the session loss still announces once");
        assert_eq!(AlarmReadError::SignedOut.to_string(), "not signed in");
        assert_eq!(AlarmReadError::Cloud("500".to_string()).to_string(), "500");
    }
}
