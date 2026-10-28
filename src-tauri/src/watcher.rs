use std::sync::Arc;
use std::time::Duration;

use proview_core::session::Store;
use proview_server::desktop::DesktopAlarms;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::notify;
use crate::shell::DesktopShell;

const BASELINE_HOURS: i64 = 1;
const OVERLAP_SECS: i64 = 120;

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
        let mut asked = false;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let settings = shell.current();
            if !settings.alarm_notify {
                asked = false;
                due = tokio::time::Instant::now();
                continue;
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
            let (since, baseline) = match shell.state.lock() {
                Ok(state) => {
                    if state.last_seen > 0 {
                        ((state.last_seen - OVERLAP_SECS).max(0), false)
                    } else {
                        (now_seconds() - BASELINE_HOURS * 3600, true)
                    }
                }
                Err(_) => continue,
            };
            match alarms.since(&sessions, since).await {
                Ok(entries) => {
                    let fresh = match shell.state.lock() {
                        Ok(mut state) => state.absorb(&entries, !baseline),
                        Err(_) => continue,
                    };
                    shell.save();
                    for alarm in fresh {
                        notify::post(&app, &alarm);
                    }
                }
                Err(error) => tracing::debug!(%error, "alarm check skipped"),
            }
        }
    });
}
