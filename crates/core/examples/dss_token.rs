use std::time::Instant;

use proview_core::cloud::dss::DssClient;
use proview_core::cloud::envelope::Identity;

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sn = std::env::args().nth(1).unwrap_or_default();
    if sn.trim().is_empty() {
        return Err("pass the device serial as the first argument".into());
    }
    let user = env("PROVIEW_DEVICE_USER");
    let password = env("PROVIEW_DEVICE_PASS");
    let identity = Identity::resolve();
    if !identity.is_complete() {
        return Err(proview_core::cloud::MISSING_CLOUD_IDENTITY.into());
    }
    let client = DssClient::new(identity);
    let started = Instant::now();
    let token = client.token(&sn, &user, &password).await?;
    let took = started.elapsed();
    match token.claims() {
        Some(claims) => println!(
            "token   {:>6.2}s app={} sn={} service={} media={} expires={} tag={}",
            took.as_secs_f64(),
            claims.app,
            claims.sn,
            claims.service,
            claims.media_ip,
            claims.expires,
            claims.tag
        ),
        None => println!("token   {:>6.2}s claims unreadable", took.as_secs_f64()),
    }
    Ok(())
}
