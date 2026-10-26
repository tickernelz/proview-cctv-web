#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use proview_server::ServeOptions;
use rand::RngCore;
use tauri::{WebviewUrl, WebviewWindowBuilder};

fn local_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn main() {
    if std::env::args().any(|arg| arg == "--version" || arg == "-V") {
        println!("proview-desktop {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let options = ServeOptions {
        address: "127.0.0.1".to_string(),
        port: 0,
        secret_key: Some(local_secret()),
    };
    let handle = runtime
        .block_on(proview_server::serve(options))
        .expect("local server");
    let url = format!("http://{}/", handle.address);
    tracing::info!(%url, "serving local session");

    tauri::Builder::default()
        .setup(move |app| {
            let parsed: tauri::Url = url.parse()?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(parsed))
                .title("PROVIEW CCTV")
                .inner_size(1280.0, 820.0)
                .min_inner_size(880.0, 600.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tauri application");

    handle.shutdown.notify();
    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
}
