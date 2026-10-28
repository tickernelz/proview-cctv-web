use std::path::{Path, PathBuf};

use proview_core::config::{Config, ProviderKind};
use proview_server::desktop::DesktopControl;
use proview_server::settings::SettingsStore;
use proview_server::{serve_desktop, serve_with_state, ServeOptions};
use reqwest::Client;
use rocket::Shutdown;

const TOKEN: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";
const HEADER: &str = "x-proview-desktop-token";

struct Running {
    base: String,
    shutdown: Shutdown,
}

fn config() -> Config {
    Config {
        provider: ProviderKind::Fixture,
        cloud_base: "https://pub-token.xmeye.net".to_string(),
        device_user: "admin".to_string(),
        device_pass: "device-password".to_string(),
        fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
        device_host: None,
        device_onvif: None,
        device_rtsp: None,
        session_ttl_secs: 3600,
    }
}

fn options(dir: &Path) -> ServeOptions {
    ServeOptions {
        address: "127.0.0.1".to_string(),
        port: 0,
        secret_key: Some("uVo3haa/znSxx4MPMV95m4oe9EYQe99QbYgEuBtVqqU=".to_string()),
        data_dir: Some(dir.to_path_buf()),
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("proview-desktop-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

async fn wait_ready(base: &str) {
    let client = Client::new();
    for _ in 0..100 {
        if let Ok(response) = client.get(format!("{base}/api/health")).send().await {
            if response.status().is_success() {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the server did not answer on {base}");
}

async fn start_desktop(dir: &Path) -> (Running, tokio::sync::mpsc::UnboundedReceiver<()>) {
    let (control, restarts) = DesktopControl::new(TOKEN);
    let handle = serve_desktop(
        options(dir),
        config(),
        SettingsStore::load(Some(dir)),
        control,
    )
    .await
    .expect("server");
    let base = format!("http://{}", handle.address);
    wait_ready(&base).await;
    (
        Running {
            base,
            shutdown: handle.shutdown,
        },
        restarts,
    )
}

async fn start_plain(dir: &Path) -> Running {
    let handle = serve_with_state(options(dir), config(), SettingsStore::load(Some(dir)))
        .await
        .expect("server");
    let base = format!("http://{}", handle.address);
    wait_ready(&base).await;
    Running {
        base,
        shutdown: handle.shutdown,
    }
}

#[test]
fn a_foreign_host_gets_nothing() {
    let dir = temp_dir("host");
    block_on(async {
        let (running, mut restarts) = start_desktop(&dir).await;
        let client = Client::new();
        let authority = running.base.trim_start_matches("http://").to_string();
        let foreign = format!(
            "rebind.example:{}",
            authority.rsplit(':').next().unwrap_or("80")
        );

        let refused = client
            .post(format!("{}/api/desktop/restart", running.base))
            .header("host", &foreign)
            .header(HEADER, TOKEN)
            .send()
            .await
            .expect("foreign host")
            .status();
        assert_eq!(refused, reqwest::StatusCode::FORBIDDEN);
        assert!(restarts.try_recv().is_err(), "a foreign host restarted");

        let page = client
            .get(format!("{}/", running.base))
            .header("host", &foreign)
            .send()
            .await
            .expect("page")
            .text()
            .await
            .expect("body");
        assert!(
            !page.contains("window.__PROVIEW_DESKTOP__"),
            "a foreign host read the token"
        );
        assert!(page.contains("</html>"), "the page lost its tail");

        let own = client
            .get(format!("{}/", running.base))
            .send()
            .await
            .expect("page")
            .text()
            .await
            .expect("body");
        assert!(own.contains(TOKEN), "the bound authority lost its token");

        running.shutdown.notify();
    });
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_the_page_token_starts_a_restart() {
    let dir = temp_dir("token");
    block_on(async {
        let (running, mut restarts) = start_desktop(&dir).await;
        let client = Client::new();
        let url = format!("{}/api/desktop/restart", running.base);

        let anonymous = client.post(&url).send().await.expect("anonymous").status();
        assert_eq!(anonymous, reqwest::StatusCode::FORBIDDEN);
        assert!(restarts.try_recv().is_err(), "an anonymous post restarted");

        let wrong = client
            .post(&url)
            .header(HEADER, "0".repeat(TOKEN.len()))
            .send()
            .await
            .expect("wrong token")
            .status();
        assert_eq!(wrong, reqwest::StatusCode::FORBIDDEN);
        assert!(restarts.try_recv().is_err(), "a wrong token restarted");

        let accepted = client
            .post(&url)
            .header(HEADER, TOKEN)
            .send()
            .await
            .expect("token")
            .status();
        assert_eq!(accepted, reqwest::StatusCode::ACCEPTED);
        assert!(restarts.try_recv().is_ok(), "the token did not restart");

        running.shutdown.notify();
    });
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_desktop_page_carries_its_token_and_a_server_page_does_not() {
    let dir = temp_dir("bootstrap");
    block_on(async {
        let (desktop, _restarts) = start_desktop(&dir).await;
        let client = Client::new();
        let page = client
            .get(format!("{}/", desktop.base))
            .send()
            .await
            .expect("page")
            .text()
            .await
            .expect("body");
        assert!(
            page.contains("window.__PROVIEW_DESKTOP__"),
            "no desktop bootstrap"
        );
        assert!(page.contains(TOKEN), "the bootstrap lost the token");
        assert!(page.contains("</html>"), "the page lost its tail");
        desktop.shutdown.notify();

        let plain = start_plain(&dir).await;
        let page = client
            .get(format!("{}/", plain.base))
            .send()
            .await
            .expect("page")
            .text()
            .await
            .expect("body");
        assert!(
            !page.contains("window.__PROVIEW_DESKTOP__"),
            "a server claimed desktop"
        );

        let refused = client
            .post(format!("{}/api/desktop/restart", plain.base))
            .header(HEADER, TOKEN)
            .send()
            .await
            .expect("restart")
            .status();
        assert_eq!(refused, reqwest::StatusCode::NOT_FOUND);
        plain.shutdown.notify();
    });
    let _ = std::fs::remove_dir_all(dir);
}
