use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

pub const LIVE_EVENT: &str = "desktop-live";

fn icon(app: &AppHandle) -> Option<tauri::image::Image<'_>> {
    app.default_window_icon()
        .cloned()
        .or_else(|| tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png")).ok())
}

#[cfg(target_os = "linux")]
fn tray_library() -> Option<&'static str> {
    const NAMES: [&str; 4] = [
        "libayatana-appindicator3.so.1",
        "libayatana-appindicator3.so",
        "libappindicator3.so.1",
        "libappindicator3.so",
    ];
    NAMES
        .into_iter()
        .find(|name| unsafe { libloading::Library::new(name) }.is_ok())
}

#[cfg(not(target_os = "linux"))]
fn tray_library() -> Option<&'static str> {
    Some("system")
}

pub fn install(app: &AppHandle) -> Result<(), String> {
    let library = tray_library().ok_or_else(|| {
        "libayatana-appindicator3 is not installed, so the tray cannot be built".to_string()
    })?;
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(app)));
    match built {
        Ok(Ok(())) => {
            tracing::info!(library, "tray icon installed");
            Ok(())
        }
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("the tray backend panicked while building".to_string()),
    }
}

fn build(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(
        app,
        "status",
        "Alarm notifications off",
        false,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let live = MenuItem::with_id(app, "live", "Live", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &live,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("PROVIEW CCTV")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => focus_main(app),
            "live" => {
                focus_main(app);
                crate::notify::dispatch(app, LIVE_EVENT, ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                focus_main(tray.app_handle());
            }
        });
    if let Some(icon) = icon(app) {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    app.manage(StatusLine(status.clone()));
    Ok(())
}

pub struct StatusLine(MenuItem<tauri::Wry>);

pub fn set_status(app: &AppHandle, text: &str) {
    if let Some(line) = app.try_state::<StatusLine>() {
        if let Err(error) = line.0.set_text(text) {
            tracing::warn!(%error, "the tray status line did not take the text");
        }
    }
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    if let Err(error) = tray.set_tooltip(Some(format!("PROVIEW CCTV — {text}"))) {
        tracing::warn!(%error, "the tray tooltip did not take the text");
    }
}

pub fn focus_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

pub fn hint_once(app: &AppHandle, shell: &crate::shell::DesktopShell) {
    let first = match shell.state.lock() {
        Ok(mut state) => {
            if state.tray_hint {
                false
            } else {
                state.tray_hint = true;
                true
            }
        }
        Err(_) => false,
    };
    if !first {
        return;
    }
    shell.save();
    tracing::info!("the window moved to the tray for the first time");
    let id = 0;
    let shown = app
        .notification()
        .builder()
        .id(id)
        .title("PROVIEW CCTV is still running")
        .body("The window is in the tray. Open it from the tray icon, or Quit from its menu.")
        .show();
    if let Err(error) = shown {
        tracing::warn!(%error, "the system did not take the notification");
    }
}
