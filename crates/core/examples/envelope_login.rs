use std::time::Instant;

use proview_core::cloud::envelope::{claims, EnvelopeClient, Identity};

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
    let identity = Identity::resolve();
    if !identity.is_complete() {
        return Err(proview_core::cloud::MISSING_CLOUD_IDENTITY.into());
    }
    let client = EnvelopeClient::new(identity);
    let overall = Instant::now();

    let started = Instant::now();
    let tokens = client.login_envelope(&account, &password).await?;
    let login = started.elapsed();
    match claims(&tokens.access) {
        Some(claims) => println!(
            "login   {:>6.2}s access={} refresh={} exp={} account={}",
            login.as_secs_f64(),
            short(&tokens.access),
            short(&tokens.refresh),
            claims.exp.unwrap_or_default(),
            claims.account.unwrap_or_default()
        ),
        None => println!(
            "login   {:>6.2}s access={}",
            login.as_secs_f64(),
            short(&tokens.access)
        ),
    }

    let device_id = {
        let raw = env("PROVIEW_DEVICE_ID");
        if raw.is_empty() {
            "00000000000000000000".to_string()
        } else {
            raw
        }
    };
    let started = Instant::now();
    let device_token = client.device_token(&tokens.access, &device_id).await?;
    if let Ok(path) = std::env::var("PROVIEW_TOKEN_DUMP") {
        std::fs::write(&path, &device_token)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    println!(
        "token   {:>6.2}s device token {}",
        started.elapsed().as_secs_f64(),
        short(&device_token)
    );

    let started = Instant::now();
    let devices = client.device_list(&tokens.access).await?;
    println!(
        "devices {:>6.2}s count={}",
        started.elapsed().as_secs_f64(),
        devices.len()
    );
    for device in &devices {
        println!(
            "        uuid={} name={} owner={} channels={}",
            device.uuid,
            device.display_name(),
            device.username,
            device.channel_count()
        );
    }
    let started = Instant::now();
    match client.refresh(&tokens.refresh).await {
        Ok(next) => println!(
            "refresh {:>6.2}s access={} same={} exp={}",
            started.elapsed().as_secs_f64(),
            short(&next.access),
            next.access == tokens.access,
            next.expires_at.unwrap_or_default()
        ),
        Err(error) => println!(
            "refresh {:>6.2}s skipped: {error}",
            started.elapsed().as_secs_f64()
        ),
    }
    println!("total   {:>6.2}s", overall.elapsed().as_secs_f64());
    Ok(())
}

fn short(token: &str) -> String {
    if token.is_empty() {
        return "-".to_string();
    }
    format!("{}..{}", &token[..6.min(token.len())], token.len())
}
