use proview_core::cloud::client::CloudClient;
use serde_json::json;

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let account = env("PROVIEW_EMAIL");
    let password = env("PROVIEW_PASSWORD");
    if account.is_empty() || password.is_empty() {
        return Err("set PROVIEW_EMAIL and PROVIEW_PASSWORD".into());
    }
    let client = CloudClient::from_env();
    let session = client.login(&account, &password, "", "").await?;
    let body = json!({ "Authorization": session.token() });
    let raw: serde_json::Value = client
        .raw_call("/api/transfer/mdlist/v1", body, Some(session.token()), None)
        .await?;
    println!("{}", serde_json::to_string_pretty(&raw)?);
    Ok(())
}
