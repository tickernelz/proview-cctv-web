use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use crate::shell::DesktopShell;

pub const LIVE_EVENT: &str = "desktop://live";

fn icon(app: &AppHandle) -> Option<tauri::image::Image<'static>> {
    app.default_window_icon()
        .cloned()
        .or_else(|| tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png")).ok())
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let live = MenuItem::with_id(app, "live", "Live", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &live, &PredefinedMenuItem::separator(app)?, &quit],
    )?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("PROVIEW CCTV")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => focus_main(app),
            "live" => {
                focus_main(app);
                if let Err(error) = app.emit_to("main", LIVE_EVENT, ()) {
                    tracing::warn!(%error, "could not reach the window");
                }
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
    Ok(())
}

pub fn focus_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

pub fn hint_once(app: &AppHandle, shell: &DesktopShell) {
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
    crate::notify::post_message(
        app,
        "PROVIEW CCTV is still running",
        "The window is in the tray. Open it from the tray icon, or Quit from its menu.",
    );
}
