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
}

impl Run {
    pub fn new(session_start: i64) -> Self {
        Self {
            session_start,
            first_check: true,
        }
    }

    pub fn policy(&self, state: &DesktopState, interval: u64) -> Policy {
        if !state.baseline_done {
            return Policy::Silent;
        }
        if self.first_check {
            return Policy::Notify {
                cutoff: self.session_start - interval as i64 - OVERLAP_SECS,
            };
        }
        Policy::Notify { cutoff: i64::MIN }
    }

    pub fn finished(&mut self, ok: bool) {
        if ok {
            self.first_check = false;
        }
    }
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
            let (since, policy) = match shell.state.lock() {
                Ok(state) => (
                    if state.last_seen > 0 {
                        (state.last_seen - OVERLAP_SECS).max(0)
                    } else {
                        now - BASELINE_HOURS * 3600
                    },
                    run.policy(&state, settings.interval_secs),
                ),
                Err(_) => continue,
            };
            let floor = since - PAD_SECS - 3600;
            match alarms.since(&sessions, since).await {
                Ok(entries) => {
                    let batch = match shell.state.lock() {
                        Ok(mut state) => state.absorb(&entries, policy, floor, now),
                        Err(_) => continue,
                    };
                    run.finished(true);
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
                    run.finished(false);
                    tracing::debug!(%error, "alarm check skipped");
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

    #[test]
    fn a_failed_first_check_does_not_arm_the_flood_gate() {
        let mut state = DesktopState {
            baseline_done: true,
            last_seen: 9_000,
            ..DesktopState::default()
        };
        let mut run = Run::new(10_000);
        run.finished(false);
        let policy = run.policy(&state, 60);
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
        run.finished(true);
        assert_eq!(
            run.policy(&state, 60),
            Policy::Notify { cutoff: i64::MIN },
            "after a successful check the run is awake"
        );
    }

    #[test]
    fn an_empty_baseline_pass_lets_the_next_alarm_through() {
        let mut state = DesktopState::default();
        let mut run = Run::new(10_000);
        let policy = run.policy(&state, 60);
        assert_eq!(policy, Policy::Silent, "a fresh install records quietly");
        let quiet = state.absorb(&[], policy, 0, 10_000);
        assert!(quiet.fresh.is_empty());
        run.finished(true);
        assert_eq!(
            run.policy(&state, 60),
            Policy::Notify { cutoff: i64::MIN },
            "the empty baseline pass still ends the baseline"
        );
        let batch = state.absorb(
            &[alarm("first", 10_010)],
            Policy::Notify { cutoff: i64::MIN },
            0,
            10_010,
        );
        assert_eq!(batch.fresh.len(), 1, "the first real alarm is announced");
    }

    #[test]
    fn a_switch_flip_starts_a_run_with_the_gate_armed() {
        let state = DesktopState {
            baseline_done: true,
            last_seen: 9_000,
            ..DesktopState::default()
        };
        let run = Run::new(50_000);
        assert_eq!(
            run.policy(&state, 15),
            Policy::Notify {
                cutoff: 50_000 - 15 - OVERLAP_SECS
            },
            "turning the switch on is a run start, not a wake-up"
        );
    }
}
