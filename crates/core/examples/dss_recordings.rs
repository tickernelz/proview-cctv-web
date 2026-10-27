use std::time::Instant;

use proview_core::cloud::envelope::{claims, EnvelopeClient, Identity};
use serde_json::json;

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

fn field(device: &proview_core::cloud::CloudDevice, key: &str) -> String {
    device
        .extra
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let day = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "2026-10-27".to_string());
    let client = EnvelopeClient::new(Identity::resolve());
    let started = Instant::now();
    let tokens = client
        .login_envelope(&env("PROVIEW_EMAIL"), &env("PROVIEW_PASSWORD"))
        .await?;
    let devices = client.device_list(&tokens.access).await?;
    let user = claims(&tokens.access)
        .and_then(|c| c.account)
        .unwrap_or_default();
    let device = devices.first().ok_or("no device on this account")?;
    let login = started.elapsed();
    let body = json!({
        "msg": "video_query",
        "sn": device.uuid,
        "st": format!("{day} 00:00:00"),
        "et": format!("{day} 23:59:59"),
        "auth": field(device, "pms"),
        "ch": 0,
        "muserid": user,
    });
    let http = reqwest::Client::builder()
        .user_agent("proview-cctv-web/0.3")
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let query = Instant::now();
    let reply = http
        .post("http://access-pms.secu100.net:6602/v2")
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await?;
    let status = reply.status();
    let text = reply.text().await.unwrap_or_default();
    println!(
        "login {:.2}s device={} query {:.2}s {status} code={}",
        login.as_secs_f64(),
        device.uuid,
        query.elapsed().as_secs_f64(),
        reply_code(&text)
    );
    if std::env::var("PROVIEW_RAW").is_ok() {
        println!("{text}");
    }
    Ok(())
}

fn reply_code(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get("code")
                .and_then(|code| code.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| text.chars().take(40).collect())
}
