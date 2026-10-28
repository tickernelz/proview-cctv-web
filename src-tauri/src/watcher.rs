use std::sync::Arc;
use std::time::Duration;

use proview_core::session::Store;
use proview_server::desktop_alarms::{AlarmReadError, DesktopAlarms};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::desktop_state::{now_seconds, Policy, Run, OVERLAP_SECS};
use crate::notify;
use crate::shell::DesktopShell;

const BASELINE_HOURS: i64 = 1;
const PAD_SECS: i64 = 14 * 3600;

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
                shell.restart_alarm_run(now_seconds());
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
                    let mut alarm = match shell.alarm.lock() {
                        Ok(alarm) => alarm,
                        Err(_) => continue,
                    };
                    if let Some(name) = account.as_deref() {
                        let previous = state.account.clone();
                        if state.switch_account(name) {
                            if !previous.is_empty() {
                                tracing::info!(
                                    from = %previous,
                                    to = %name,
                                    "another account signed in; the alarm run starts over"
                                );
                            }
                            *alarm = Run::new(now);
                        }
                    }
                    (
                        if state.last_seen > 0 {
                            (state.last_seen - OVERLAP_SECS).max(0)
                        } else {
                            now - BASELINE_HOURS * 3600
                        },
                        alarm.policy(&state, settings.interval_secs, now),
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
                    if let Ok(mut alarm) = shell.alarm.lock() {
                        alarm.finished(true, now);
                    }
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
                    let lost = matches!(error, AlarmReadError::SignedOut);
                    let announced = match shell.alarm.lock() {
                        Ok(mut alarm) => alarm.failed(lost),
                        Err(_) => false,
                    };
                    if announced {
                        notify::post_paused(&app);
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
    use crate::desktop_state::DesktopState;
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
        assert!(run.failed(true));
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
    fn a_hide_before_any_success_announces_the_pause_once() {
        let mut run = Run::new(0);
        assert!(
            run.tray_pause_notice(),
            "sign-in never happened, so the pause is the truth"
        );
        assert!(
            !run.tray_pause_notice(),
            "a second hide in the same run stays quiet"
        );
        run.finished(true, 10);
        assert!(
            !run.tray_pause_notice(),
            "a run whose check succeeded is not paused"
        );
        assert!(
            run.failed(true),
            "the watcher still owns the pause once the session returns"
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
