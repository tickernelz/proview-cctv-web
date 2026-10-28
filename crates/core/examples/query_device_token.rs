use std::time::Instant;

use proview_core::cloud::envelope::{EnvelopeClient, Identity};

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
    let candidates: Vec<String> = env("PROVIEW_TOKEN_UUIDS")
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect();
    if candidates.is_empty() {
        return Err("set PROVIEW_TOKEN_UUIDS to a comma separated list".into());
    }
    let client = EnvelopeClient::new(Identity::resolve());
    let tokens = client.login_envelope(&account, &password).await?;
    for candidate in &candidates {
        let started = Instant::now();
        let outcome = client.device_login_token(&tokens.access, candidate).await;
        let elapsed = started.elapsed().as_secs_f64();
        let label = if candidate.len() > 6 {
            format!("{}..", &candidate[..6])
        } else {
            candidate.clone()
        };
        match outcome {
            Ok(value) => println!(
                "{:<10} {:>5.2}s issued len={}",
                label,
                elapsed,
                value.chars().count()
            ),
            Err(error) => println!("{:<10} {:>5.2}s {error}", label, elapsed),
        }
    }
    Ok(())
}
