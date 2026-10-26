#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod keychain;

use proview_core::config::Config;
use proview_server::ServeOptions;
use rand::RngCore;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

use crate::keychain::{Credentials, OsKeychain, SecretStore};

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

fn main() {
    if flag("--version") || flag("-V") {
        println!("proview-desktop {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    if let Some(code) = credential_command() {
        std::process::exit(code);
    }

    let config = config_from_keychain();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let handle = runtime.handle().clone();

    tauri::Builder::default()
        .setup(move |app| {
            let data_dir = app.path().app_data_dir().ok();
            let options = ServeOptions {
                address: "127.0.0.1".to_string(),
                port: 0,
                secret_key: Some(local_secret()),
                data_dir,
            };
            let server = handle.block_on(proview_server::serve_with(options, config.clone()))?;
            let url: tauri::Url = format!("http://{}/", server.address).parse()?;
            tracing::info!(%url, "serving the local session");
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("PROVIEW CCTV")
                .inner_size(1280.0, 820.0)
                .min_inner_size(880.0, 600.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tauri application");

    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
}
