#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod keychain;

use proview_core::config::Config;
use proview_server::ServeOptions;
use rand::RngCore;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tracing_subscriber::prelude::*;

use crate::keychain::{Credentials, OsKeychain, SecretStore};

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

const RESTART_PROBE: &str = include_str!("restart_probe.js");

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

fn main() {
    if flag("--version") || flag("-V") {
        println!("proview-desktop {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if let Some(code) = credential_command() {
        std::process::exit(code);
    }

    let probe = restart_probe();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let handle = runtime.handle().clone();

    tauri::Builder::default()
        .setup(move |app| {
            let data_dir = app.path().app_data_dir().ok();
            init_log(data_dir.as_deref());
            tracing::info!(data = ?data_dir, version = env!("CARGO_PKG_VERSION"), "starting");
            let settings = proview_server::settings::SettingsStore::load(data_dir.as_deref());
            settings.apply_env();
            let config = config_from_keychain();
            let options = ServeOptions {
                address: "127.0.0.1".to_string(),
                port: 0,
                secret_key: Some(local_secret()),
                data_dir,
            };
            let (control, mut restarts) =
                proview_server::desktop::DesktopControl::new(local_secret());
            let server = handle.block_on(proview_server::serve_desktop(
                options, config, settings, control,
            ))?;
            let url: tauri::Url = format!("http://{}/", server.address).parse()?;
            tracing::info!(%url, "serving the local session");
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
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
            if probe.is_some() {
                let window = window.clone();
                handle.spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                    tracing::info!("probe: driving the Restart the app button");
                    if let Err(error) = window.eval(RESTART_PROBE) {
                        tracing::error!(%error, "probe: the driver did not run");
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tauri application");

    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
}
