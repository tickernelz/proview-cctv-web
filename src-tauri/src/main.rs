#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop_settings;
mod desktop_state;
mod keychain;
mod notify;
mod shell;
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

#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    app.restart();
}

fn main() {
    if flag("--version") || flag("-V") {
        println!("proview-desktop {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if let Some(code) = credential_command() {
        std::process::exit(code);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let handle = runtime.handle().clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            let data_dir = app.path().app_data_dir().ok();
            init_log(data_dir.as_deref());
            tracing::info!(data = ?data_dir, version = env!("CARGO_PKG_VERSION"), "starting");
            let settings = SettingsStore::load(data_dir.as_deref());
            settings.apply_env();
            let config = config_from_keychain();
            let options = ServeOptions {
                address: "127.0.0.1".to_string(),
                port: 0,
                secret_key: Some(local_secret()),
                data_dir: data_dir.clone(),
            };
            let (server, sessions) = handle.block_on(proview_server::serve_shared(
                options,
                config,
                settings.clone(),
            ))?;
            let url: tauri::Url = format!("http://{}/", server.address).parse()?;
            tracing::info!(%url, "serving the local session");
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("PROVIEW CCTV")
                .inner_size(1280.0, 820.0)
                .min_inner_size(880.0, 600.0)
                .build()?;
            let state_path = data_dir.map(|dir| dir.join("desktop-state.json"));
            let shell = Arc::new(DesktopShell::new(settings, state_path));
            match notify::install(app.handle()) {
                Ok(()) => tracing::info!("notification clicks are wired to the window"),
                Err(error) => tracing::warn!(%error, "notifications are unavailable"),
            }
            match tray::install(app.handle()) {
                Ok(()) => {
                    shell.set_tray_ready(true);
                    tracing::info!("tray icon installed");
                }
                Err(error) => tracing::warn!(%error, "no system tray; the window closes normally"),
            }
            app.manage(shell.clone());
            watcher::spawn(&handle, app.handle().clone(), sessions, shell);
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
                        tray::hint_once(window.app_handle(), &shell);
                    }
                }
                WindowEvent::Resized(_) => {
                    if settings.minimize_to_tray
                        && shell.tray_ready()
                        && window.is_minimized().unwrap_or(false)
                    {
                        let _ = window.hide();
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![restart_app])
        .run(tauri::generate_context!())
        .expect("tauri application");

    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
}
