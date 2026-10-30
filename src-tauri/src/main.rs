#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop_settings;
mod desktop_state;
mod keychain;
mod notify;
mod shell;
mod status;
mod tray;
mod watcher;

use std::sync::Arc;

use proview_core::config::Config;
use proview_server::settings::SettingsStore;
use proview_server::ServeOptions;
use rand::RngCore;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tracing_subscriber::prelude::*;

use crate::keychain::{Credentials, OsKeychain, SecretStore};
use crate::shell::DesktopShell;

fn init_log(dir: Option<&std::path::Path>) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let stderr = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
    let registry = tracing_subscriber::registry().with(filter).with(stderr);
    match dir {
        Some(dir) => {
            let _ = std::fs::create_dir_all(dir);
            let appender = tracing_appender::rolling::never(dir, "desktop.log");
            let file = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(appender);
            let _ = registry.with(file).try_init();
        }
        None => {
            let _ = registry.try_init();
        }
    }
}

fn local_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn flag(name: &str) -> bool {
    std::env::args().any(|item| item == name)
}

fn profile() -> String {
    std::env::var("PROVIEW_PROFILE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| keychain::DEFAULT_PROFILE.to_string())
}

fn credential_command() -> Option<i32> {
    let keychain = OsKeychain;
    let profile = profile();
    if flag("--store-credentials") {
        let credentials: Credentials = match keychain::read_stdin_credentials() {
            Ok(credentials) => credentials,
            Err(error) => {
                eprintln!("could not read credentials: {error}");
                return Some(2);
            }
        };
        return Some(match keychain.put(&profile, &credentials) {
            Ok(()) => {
                println!("stored the device credentials for profile {profile}");
                0
            }
            Err(error) => {
                eprintln!("could not store the device credentials: {error}");
                1
            }
        });
    }
    if flag("--check-credentials") {
        return Some(match keychain.get(&profile) {
            Ok(Some(credentials)) => {
                println!(
                    "profile {profile} has device credentials for user {}",
                    credentials.user
                );
                0
            }
            Ok(None) => {
                println!("profile {profile} has no device credentials");
                0
            }
            Err(error) => {
                eprintln!("could not read the device credentials: {error}");
                1
            }
        });
    }
    if flag("--forget-credentials") {
        return Some(match keychain.delete(&profile) {
            Ok(()) => {
                println!("forgot the device credentials for profile {profile}");
                0
            }
            Err(error) => {
                eprintln!("could not forget the device credentials: {error}");
                1
            }
        });
    }
    None
}

fn config_from_keychain() -> Config {
    let mut config = Config::from_env();
    match OsKeychain.get(&profile()) {
        Ok(Some(credentials)) => {
            config.device_user = credentials.user;
            config.device_pass = credentials.password;
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "no OS keychain available; using the environment"),
    }
    config
}

#[cfg(feature = "restart-probe")]
const RESTART_PROBE: &str = include_str!("restart_probe.js");

#[cfg(feature = "restart-probe")]
fn restart_probe() -> Option<std::path::PathBuf> {
    if !flag("--restart-probe") {
        return None;
    }
    let marker = std::env::var("PROVIEW_RESTART_PROBE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("proview-restart-probe.marker"));
    if marker.exists() {
        let _ = std::fs::remove_file(&marker);
        println!("restart probe: the app came back, app.restart() ran");
        std::process::exit(0);
    }
    match std::fs::write(&marker, std::process::id().to_string()) {
        Ok(()) => println!("restart probe: marker {} written", marker.display()),
        Err(error) => eprintln!("restart probe: no marker at {}: {error}", marker.display()),
    }
    Some(marker)
}

const WEB_DEV_API_PORT: u16 = 8787;
const SAVED_PORT_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

fn page_port(state_path: Option<&std::path::Path>) -> u16 {
    let Some(path) = state_path else {
        return proview_server::choose_port("127.0.0.1", 0, SAVED_PORT_WAIT).port;
    };
    let mut state = desktop_state::DesktopState::load(path);
    let choice = proview_server::choose_port("127.0.0.1", state.port, SAVED_PORT_WAIT);
    if choice.fell_back {
        tracing::warn!(
            saved = state.port,
            port = choice.port,
            "the saved page port is taken; this launch uses another one and the page preferences start fresh"
        );
    }
    if state.port != choice.port {
        state.port = choice.port;
        if let Err(error) = state.save(path) {
            tracing::warn!(%error, "could not save the page port");
        }
    }
    choice.port
}

fn web_dev_url(app: &tauri::App) -> Option<tauri::Url> {
    if !cfg!(debug_assertions) || std::env::var_os("PROVIEW_WEB_DEV").is_none() {
        return None;
    }
    app.config().build.dev_url.clone()
}

fn main() {
    if flag("--version") || flag("-V") {
        println!("proview-desktop {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if let Some(code) = credential_command() {
        std::process::exit(code);
    }

    #[cfg(feature = "restart-probe")]
    let probe = restart_probe();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let handle = runtime.handle().clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::focus_main(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            let data_dir = app.path().app_data_dir().ok();
            init_log(data_dir.as_deref());
            tracing::info!(data = ?data_dir, version = env!("CARGO_PKG_VERSION"), "starting");
            let settings = SettingsStore::load(data_dir.as_deref());
            settings.apply_env();
            let config = config_from_keychain();
            let web_dev = web_dev_url(app);
            let state_path = data_dir.as_ref().map(|dir| dir.join("desktop-state.json"));
            let options = ServeOptions {
                address: "127.0.0.1".to_string(),
                port: if web_dev.is_some() {
                    WEB_DEV_API_PORT
                } else {
                    page_port(state_path.as_deref())
                },
                secret_key: Some(local_secret()),
                data_dir,
            };
            let (control, mut restarts) =
                proview_server::desktop::DesktopControl::new(local_secret());
            let (server, sessions) = handle.block_on(proview_server::serve_desktop_shared(
                options,
                config,
                settings.clone(),
                control,
            ))?;
            let url: tauri::Url = match web_dev {
                Some(url) => url,
                None => format!("http://{}/", server.address).parse()?,
            };
            tracing::info!(%url, "serving the local session");
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("PROVIEW CCTV")
                .inner_size(1280.0, 820.0)
                .min_inner_size(880.0, 600.0)
                .build()?;
            let quitting = app.handle().clone();
            handle.spawn(async move {
                if restarts.recv().await.is_some() {
                    tracing::info!("the page asked for a restart");
                    quitting.restart();
                }
            });
            let shell = Arc::new(DesktopShell::new(settings, state_path, sessions));
            match notify::install(app.handle()) {
                Ok(()) => tracing::info!("notification clicks are wired to the window"),
                Err(error) => tracing::warn!(%error, "notifications are unavailable"),
            }
            match tray::install(app.handle()) {
                Ok(()) => shell.set_tray_ready(true),
                Err(reason) => {
                    tracing::warn!(%reason, "no system tray; the window closes normally")
                }
            }
            app.manage(shell.clone());
            watcher::spawn(&handle, app.handle().clone(), shell);
            #[cfg(feature = "restart-probe")]
            if probe.is_some() {
                let main = app.get_webview_window("main");
                handle.spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                    tracing::info!("probe: driving the Restart the app button");
                    match main {
                        Some(window) => {
                            if let Err(error) = window.eval(RESTART_PROBE) {
                                tracing::error!(%error, "probe: the driver did not run");
                            }
                        }
                        None => tracing::error!("probe: no window to drive"),
                    }
                });
            }
            if flag("--notify-probe") {
                let app = app.handle().clone();
                notify::post_message(&app, "PROVIEW CCTV", "Notification probe");
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    app.exit(0);
                });
            }
            if flag("--event-probe") {
                let app = app.handle().clone();
                std::thread::spawn(move || probe_page(&app));
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            let Some(state) = window.try_state::<Arc<DesktopShell>>() else {
                return;
            };
            let shell = state.inner().clone();
            let settings = shell.current();
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    if settings.close_to_tray && shell.tray_ready() {
                        api.prevent_close();
                        let _ = window.hide();
                        shell.refresh_status(window.app_handle(), settings.alarm_notify);
                        if !tray_pause(window.app_handle(), &shell, settings.alarm_notify) {
                            tray::hint_once(window.app_handle(), &shell);
                        }
                    }
                }
                WindowEvent::Resized(_)
                    if settings.minimize_to_tray
                        && shell.tray_ready()
                        && window.is_minimized().unwrap_or(false) =>
                {
                    let _ = window.hide();
                    shell.refresh_status(window.app_handle(), settings.alarm_notify);
                    tray_pause(window.app_handle(), &shell, settings.alarm_notify);
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())
        .expect("tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                tray::focus_main(app);
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (app, event);
            }
        });

    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
}

fn tray_pause(app: &tauri::AppHandle, shell: &DesktopShell, notify_on: bool) -> bool {
    if !notify_on || !shell.take_tray_pause_notice() {
        return false;
    }
    notify::post_paused(app);
    true
}

fn probe_page(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    std::thread::sleep(std::time::Duration::from_secs(6));
    let armed = window.eval(
        "window.__proviewProbe = 'none'; window.addEventListener('desktop-alarm', (event) => { window.__proviewProbe = event.detail && event.detail.id ? 'seen:' + event.detail.id : 'seen:no-id'; });",
    );
    if let Err(error) = armed {
        tracing::warn!(%error, "the page did not accept the probe listener");
        return;
    }
    let alarm = proview_server::desktop_alarms::DesktopAlarm {
        id: "probe".to_string(),
        serial: "PROBE0001".to_string(),
        channel: "0".to_string(),
        device: "Probe camera".to_string(),
        event: "Probe".to_string(),
        time: "2026-10-28 00:00:00".to_string(),
        utc_time: 0,
    };
    crate::notify::dispatch(app, crate::notify::ALARM_EVENT, &alarm);
    std::thread::sleep(std::time::Duration::from_secs(2));
    let _ = window.eval(
        "location.hash = '#proview-probe-' + window.__proviewProbe + '-tauri-' + (typeof window.__TAURI__);",
    );
    std::thread::sleep(std::time::Duration::from_secs(2));
    match window.url() {
        Ok(url) => tracing::info!(%url, "event probe finished"),
        Err(error) => tracing::warn!(%error, "the page address could not be read"),
    }
    app.exit(0);
}
