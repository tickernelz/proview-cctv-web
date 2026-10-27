use std::time::Instant;

use proview_core::config::Config;
use proview_core::provider::cloud::CloudProvider;
use proview_core::provider::Provider;

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
    let provider = CloudProvider::new(&Config::from_env());
    let started = Instant::now();
    let session = provider.login(&account, &password).await?;
    println!("login {:.2}s", started.elapsed().as_secs_f64());
    for round in 1..=3 {
        let started = Instant::now();
        let devices = provider.devices(&session).await?;
        let online = devices.iter().filter(|device| device.online).count();
        println!(
            "devices round {round}: {:.3}s count={} online={}",
            started.elapsed().as_secs_f64(),
            devices.len(),
            online
        );
    }
    Ok(())
}
