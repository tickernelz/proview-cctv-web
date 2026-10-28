use proview_core::cloud::attributes::{trans_key_of, AttributeClient};
use proview_core::cloud::client::CloudClient;
use proview_core::cloud::Identity;

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

fn redact(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => {
            let short: String = text.chars().take(2).collect();
            serde_json::Value::String(format!("{short}...len={}", text.chars().count()))
        }
        serde_json::Value::Array(items) => items.iter().map(redact).collect(),
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, item) in map {
                if key.to_lowercase().contains("key") || key.to_lowercase().contains("token") {
                    out.insert(
                        key.clone(),
                        serde_json::Value::String(format!("<len={}>", item.to_string().len())),
                    );
                } else {
                    out.insert(key.clone(), redact(item));
                }
            }
            serde_json::Value::Object(out)
        }
        other => other.clone(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let account = env("PROVIEW_EMAIL");
    let password = env("PROVIEW_PASSWORD");
    let client = CloudClient::from_env();
    let code = std::env::var("PROVIEW_LOGIN_CODE").unwrap_or_default();
    let token = std::fs::read_to_string("/tmp/xm-captcha.token").unwrap_or_default();
    let session = client
        .login(&account, &password, code.trim(), token.trim())
        .await?;
    let access = session.token().to_string();
    let attributes = AttributeClient::new(Identity::resolve());
    let pending = attributes.share_pending(&access, &[]).await?;
    println!(
        "share_pending {}",
        serde_json::to_string(&redact(&pending))?
    );
    match trans_key_of(&pending) {
        Some(key) => {
            println!("trans_key present len={}", key.chars().count());
            let info = attributes.share_info(&access, &key).await?;
            println!("share_info {}", serde_json::to_string(&redact(&info))?);
            let accepted = attributes.share_accept(&access, &key).await?;
            println!(
                "share_accept {}",
                serde_json::to_string(&redact(&accepted))?
            );
        }
        None => println!("trans_key absent"),
    }
    let devices = client.devices(&session).await?;
    println!("devices count={}", devices.len());
    for device in &devices {
        println!("  {} | {}", device.uuid, device.nickname);
    }
    Ok(())
}
