use std::sync::Arc;
use std::time::Duration;

use proview_core::session::Store;
use proview_server::desktop_alarms::DesktopAlarms;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::desktop_state::{DesktopState, Policy, OVERLAP_SECS};
use crate::notify;
use crate::shell::DesktopShell;

const BASELINE_HOURS: i64 = 1;
const PAD_SECS: i64 = 14 * 3600;

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

pub struct Run {
    session_start: i64,
    first_check: bool,
    last_ok: Option<i64>,
    failing_since: Option<i64>,
    pause_posted: bool,
}

impl Run {
    pub fn new(session_start: i64) -> Self {
        Self {
            session_start,
            first_check: true,
            last_ok: None,
            failing_since: None,
            pause_posted: false,
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
        if self.failing_since.is_some() && self.outage_is_long(interval, now) {
            return Policy::Notify { cutoff: now };
        }
        Policy::Notify { cutoff: i64::MIN }
    }

    fn outage_is_long(&self, interval: u64, now: i64) -> bool {
        self.last_ok
            .is_none_or(|ok| now - ok > 3 * interval as i64)
    }

    pub fn failed(&mut self, now: i64, session_lost: bool) -> bool {
        self.failing_since.get_or_insert(now);
        if session_lost && !self.pause_posted {
            self.pause_posted = true;
            return true;
        }
        false
    }

    pub fn finished(&mut self, ok: bool, now: i64) {
        if ok {
            self.first_check = false;
            self.failing_since = None;
            self.pause_posted = false;
            self.last_ok = Some(now);
        }
    }
}

fn session_lost(error: &str) -> bool {
    let text = error.to_ascii_lowercase();
    text.contains("not signed in") || text.contains("sign in")
}

pub fn spawn(
    handle: &tokio::runtime::Handle,
    app: AppHandle,
    sessions: Store,
    shell: Arc<DesktopShell>,
) {
    handle.spawn(async move {
        let alarms = DesktopAlarms::from_env();
        let mut due = tokio::time::Instant::now();
        let mut enabled = false;
        let mut asked = false;
        let mut run = Run::new(now_seconds());
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let settings = shell.current();
            if !settings.alarm_notify {
                if enabled {
                    tracing::info!("alarm notifications switched off");
                }
                enabled = false;
                asked = false;
                due = tokio::time::Instant::now();
                continue;
            }
            if !enabled {
                enabled = true;
                run = Run::new(now_seconds());
                due = tokio::time::Instant::now();
            }
            if !asked {
                asked = true;
                match app.notification().request_permission() {
                    Ok(state) => {
                        tracing::info!(?state, "alarm notifications switched on; the system owns this permission")
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the system did not answer the notification permission")
                    }
                }
            }
            if tokio::time::Instant::now() < due {
                continue;
            }
            due = tokio::time::Instant::now() + Duration::from_secs(settings.interval_secs);
            let now = now_seconds();
            let account = sessions.latest().map(|(_, session)| session.account);
            let (since, policy) = match shell.state.lock() {
                Ok(mut state) => {
                    if account.as_deref().is_some_and(|name| state.switch_account(name)) {
                        tracing::info!("another account signed in; the alarm run starts over");
                        run = Run::new(now);
                    }
                    (
                        if state.last_seen > 0 {
                            (state.last_seen - OVERLAP_SECS).max(0)
                        } else {
                            now - BASELINE_HOURS * 3600
                        },
                        run.policy(&state, settings.interval_secs, now),
                    )
                }
                Err(_) => continue,
            };
            let floor = since - PAD_SECS - 3600;
            match alarms.since(&sessions, since).await {
                Ok(entries) => {
                    let batch = match shell.state.lock() {
                        Ok(mut state) => state.absorb(&entries, policy, floor, now),
                        Err(_) => continue,
                    };
                    run.finished(true, now);
                    if batch.changed {
                        shell.save();
                    }
                    for alarm in &batch.fresh {
                        notify::post(&app, alarm);
                    }
                    if batch.skipped > 0 && policy != Policy::Silent {
                        notify::summary(&app, batch.skipped);
                    }
                }
                Err(error) => {
                    let lost = session_lost(&error);
                    if run.failed(now, lost) {
                        notify::post_message(
                            &app,
                            "Alarm notifications paused",
                            "Open PROVIEW and sign in to keep them coming.",
                        );
                        tracing::warn!(%error, "alarm notifications paused");
                    } else if lost {
                        tracing::debug!(%error, "alarm check skipped while the session is gone");
                    } else {
                        tracing::warn!(%error, "alarm check failed; the next interval retries");
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use proview_server::desktop_alarms::DesktopAlarm;

    fn alarm(id: &str, utc_time: i64) -> DesktopAlarm {
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
            .map(|index| alarm(&format!("m{index}"), 9_000 + index))
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
            &[alarm("first", 10_010)],
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
    fn a_long_outage_summarizes_the_gap_instead_of_toasting_each_alarm() {
        let mut state = awake_state();
        let mut run = Run::new(1_000);
        run.finished(true, 1_000);
        assert!(run.failed(1_015, true), "the pause notice is posted once");
        assert!(!run.failed(1_060, true), "the same outage stays quiet");
        let now = 1_120;
        assert!(now - 1_000 > 3 * 15, "the outage is longer than the threshold");
        let policy = run.policy(&state, 15, now);
        assert_eq!(
            policy,
            Policy::Notify { cutoff: now },
            "the recovering check summarizes what it finds"
        );
        let gap: Vec<DesktopAlarm> = (0..15)
            .map(|index| alarm(&format!("g{index}"), 1_010 + index))
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
        assert!(run.failed(1_015, true));
        let policy = run.policy(&state, 15, 1_030);
        assert_eq!(
            policy,
            Policy::Notify { cutoff: i64::MIN },
            "one missed interval is not an outage"
        );
        let batch = state.absorb(&[alarm("fresh", 1_025)], policy, 0, 1_030);
        assert_eq!(batch.fresh.len(), 1, "a single failure keeps the toast");
        assert_eq!(batch.skipped, 0);
    }

    #[test]
    fn the_pause_notice_is_posted_once_per_failed_span() {
        let mut run = Run::new(0);
        run.finished(true, 0);
        assert!(run.failed(10, true), "the first session failure announces");
        assert!(!run.failed(20, true), "the rest of the span stays quiet");
        assert!(!run.failed(30, false), "another failure keeps the span");
        run.finished(true, 40);
        assert!(run.failed(50, true), "a new span announces again");
    }

    #[test]
    fn a_failure_that_is_not_the_session_posts_no_pause() {
        let mut run = Run::new(0);
        run.finished(true, 0);
        assert!(!run.failed(10, false), "a cloud hiccup is not a pause");
        assert!(run.failed(20, true), "the session loss still announces once");
        assert!(!session_lost("the alarm centre said 500"));
        assert!(session_lost("not signed in"));
        assert!(session_lost("alarms read the vendor cloud; sign in with the cloud account"));
    }
}
