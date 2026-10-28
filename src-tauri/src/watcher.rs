use std::sync::Arc;
use std::time::Duration;

use proview_core::session::Store;
use proview_server::desktop::DesktopAlarms;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::desktop_state::{Policy, OVERLAP_SECS};
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
        let mut first_poll = true;
        let mut session_start = now_seconds();
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let settings = shell.current();
            if !settings.alarm_notify {
                if enabled {
                    tracing::info!("alarm notifications switched off");
                }
                enabled = false;
                asked = false;
                first_poll = true;
                due = tokio::time::Instant::now();
                continue;
            }
            if !enabled {
                enabled = true;
                first_poll = true;
                session_start = now_seconds();
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
            let (since, first_ever) = match shell.state.lock() {
                Ok(state) => (
                    if state.last_seen > 0 {
                        (state.last_seen - OVERLAP_SECS).max(0)
                    } else {
                        now - BASELINE_HOURS * 3600
                    },
                    state.last_seen == 0,
                ),
                Err(_) => continue,
            };
            let policy = if first_ever {
                Policy::Silent
            } else if first_poll {
                Policy::Notify {
                    cutoff: session_start - settings.interval_secs as i64 - OVERLAP_SECS,
                }
            } else {
                Policy::Notify { cutoff: i64::MIN }
            };
            first_poll = false;
            let floor = since - PAD_SECS - 3600;
            match alarms.since(&sessions, since).await {
                Ok(entries) => {
                    let batch = match shell.state.lock() {
                        Ok(mut state) => state.absorb(&entries, policy, floor, now),
                        Err(_) => continue,
                    };
                    if batch.changed {
                        shell.save();
                    }
                    for alarm in &batch.fresh {
                        notify::post(&app, alarm);
                    }
                    if batch.skipped > 0 && !first_ever {
                        notify::summary(&app, batch.skipped);
                    }
                }
                Err(error) => tracing::debug!(%error, "alarm check skipped"),
            }
        }
    });
}
