use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use proview_server::desktop::DesktopAlarm;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::{ActionPerformed, NotificationExt};

pub const OPEN_ALARM_EVENT: &str = "desktop://alarm";

static NEXT_ID: AtomicI32 = AtomicI32::new(1);

#[derive(Default)]
pub struct Pending {
    alarms: Mutex<HashMap<i32, DesktopAlarm>>,
}

pub fn install(app: &AppHandle) -> Result<(), tauri_plugin_notification::Error> {
    app.manage(Pending::default());
    let handle = app.clone();
    app.notification().on_action(move |action| {
        if action.action_id() == "tap" {
            on_tap(&handle, action);
        }
    })
}

fn on_tap(app: &AppHandle, action: &ActionPerformed) {
    let Some(notification) = action.notification() else {
        return;
    };
    let id = notification.id();
    let alarm = app.try_state::<Pending>().and_then(|pending| {
        pending
            .alarms
            .lock()
            .ok()
            .and_then(|mut map| map.remove(&id))
    });
    crate::tray::focus_main(app);
    let Some(alarm) = alarm else {
        return;
    };
    if let Err(error) = app.emit_to("main", OPEN_ALARM_EVENT, &alarm) {
        tracing::warn!(%error, alarm = %alarm.id, "could not hand the alarm to the window");
    }
}

pub fn post(app: &AppHandle, alarm: &DesktopAlarm) {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    if let Some(pending) = app.try_state::<Pending>() {
        if let Ok(mut map) = pending.alarms.lock() {
            if map.len() >= 128 {
                map.clear();
            }
            map.insert(id, alarm.clone());
        }
    }
    let title = format!("{} · {}", alarm.device, alarm.event);
    let body = if alarm.time.trim().is_empty() {
        format!("alarm {}", alarm.id)
    } else {
        alarm.time.trim().to_string()
    };
    let shown = app
        .notification()
        .builder()
        .id(id)
        .title(title)
        .body(body)
        .show();
    tracing::info!(alarm = %alarm.id, device = %alarm.device, "notification posted");
    if let Err(error) = shown {
        tracing::warn!(%error, alarm = %alarm.id, "the system did not take the notification");
    }
}

pub fn post_message(app: &AppHandle, title: &str, body: &str) {
    let shown = app.notification().builder().title(title).body(body).show();
    tracing::info!(title, "notification posted");
    if let Err(error) = shown {
        tracing::warn!(%error, title, "the system did not take the notification");
    }
}
