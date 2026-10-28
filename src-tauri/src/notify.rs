use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use proview_server::desktop::DesktopAlarm;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::{ActionPerformed, NotificationExt};

pub const ALARM_EVENT: &str = "desktop-alarm";

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

pub fn dispatch(app: &AppHandle, name: &str, payload: impl Serialize) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let (Ok(name), Ok(payload)) = (serde_json::to_string(name), serde_json::to_string(&payload))
    else {
        return;
    };
    let script = format!("window.dispatchEvent(new CustomEvent({name}, {{ detail: {payload} }}));");
    if let Err(error) = window.eval(&script) {
        tracing::warn!(%error, event = %name, "could not hand the event to the page");
    }
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
    if let Some(alarm) = alarm {
        dispatch(app, ALARM_EVENT, alarm);
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
    show(app, id, &title, &body);
    tracing::info!(alarm = %alarm.id, device = %alarm.device, "notification posted");
}

pub fn post_message(app: &AppHandle, title: &str, body: &str) {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    show(app, id, title, body);
    tracing::info!(title, "notification posted");
}

pub fn summary(app: &AppHandle, count: usize) {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let title = format!("{count} alarm while PROVIEW was closed");
    show(app, id, &title, "Open the alarm sheet to review them.");
    tracing::info!(count, "notification posted");
}

fn show(app: &AppHandle, id: i32, title: &str, body: &str) {
    let shown = app
        .notification()
        .builder()
        .id(id)
        .title(title)
        .body(body)
        .show();
    if let Err(error) = shown {
        tracing::warn!(%error, title, "the system did not take the notification");
    }
}
