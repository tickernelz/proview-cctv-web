use std::time::Duration;

use proview_core::cloud::{EnvelopeClient, Identity};
use proview_core::provider::VendorSession;
use proview_core::session::Store;
use proview_server::desktop_alarms::{AlarmReadError, DesktopAlarms};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn stub_vendor(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("stub listener");
    let address = listener.local_addr().expect("stub address");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 4096];
                let _ = socket.read(&mut buffer).await;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://{address}")
}

fn signed_in() -> Store {
    let store = Store::new();
    store.put(
        "probe-session".to_string(),
        VendorSession {
            account: "probe@example.invalid".to_string(),
            token: Some("revoked-token".to_string()),
        },
        Duration::from_secs(60),
    );
    store
}

fn alarms_pointed_at(base: String) -> DesktopAlarms {
    DesktopAlarms::new(EnvelopeClient::with_base(
        Identity::new("probe-uuid", "probe-key", "probe-secret"),
        base,
    ))
}

#[tokio::test]
async fn a_revoked_vendor_token_reads_as_signed_out() {
    let base = stub_vendor(r#"{"code":-4004,"msg":"token revoked"}"#).await;
    let error = alarms_pointed_at(base)
        .since(&signed_in(), 1_700_000_000)
        .await
        .expect_err("a revoked token cannot read alarms");
    assert_eq!(error, AlarmReadError::SignedOut);
    assert_eq!(error.to_string(), "not signed in");
}

#[tokio::test]
async fn a_refused_device_list_stays_a_cloud_failure() {
    let base = stub_vendor(r#"{"code":-1001,"msg":"device list refused"}"#).await;
    let error = alarms_pointed_at(base)
        .since(&signed_in(), 1_700_000_000)
        .await
        .expect_err("a refused device list cannot read alarms");
    assert!(
        matches!(error, AlarmReadError::Cloud(_)),
        "the watcher warns and keeps waiting instead of announcing a pause: {error}"
    );
    assert!(error.to_string().contains("device list refused"), "{error}");
}
