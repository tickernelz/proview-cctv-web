use std::sync::Arc;
use std::time::Duration;

use proview_server::desktop_alarms::{AlarmReadError, DesktopAlarms};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::desktop_state::{now_seconds, Policy, Run, OVERLAP_SECS};
use crate::notify;
use crate::shell::DesktopShell;

const BASELINE_HOURS: i64 = 1;
const PAD_SECS: i64 = 14 * 3600;

pub fn spawn(handle: &tokio::runtime::Handle, app: AppHandle, shell: Arc<DesktopShell>) {
    handle.spawn(async move {
        let alarms = DesktopAlarms::from_env();
        let sessions = shell.sessions.clone();
        let mut due = tokio::time::Instant::now();
        let mut enabled = shell.current().alarm_notify;
        let mut asked = false;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let settings = shell.current();
            shell.refresh_status(&app, settings.alarm_notify);
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
