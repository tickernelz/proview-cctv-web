use std::time::Instant;

use proview_core::cloud::attributes::AttributeClient;
use proview_core::cloud::envelope::{EnvelopeClient, Identity};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let account = std::env::var("PROVIEW_EMAIL")?;
    let password = std::env::var("PROVIEW_PASSWORD")?;
    let identity = Identity::resolve();
    let cloud = EnvelopeClient::new(identity.clone());
    let admin = AttributeClient::new(identity);

    let started = Instant::now();
    let tokens = cloud.login_envelope(&account, &password).await?;
    println!("login            {} ms", started.elapsed().as_millis());

    let started = Instant::now();
    let devices = cloud.device_list(&tokens.access).await?;
    println!(
        "msdlistCode      {} ms ({} device)",
        started.elapsed().as_millis(),
        devices.len()
    );

    let uuid = devices[0].uuid.clone();
    let started = Instant::now();
    let pending = admin
        .share_pending(&tokens.access, std::slice::from_ref(&uuid))
        .await?;
    println!(
        "hasDevicesTrans  {} ms -> {}",
        started.elapsed().as_millis(),
        pending["data"]
    );

    let started = Instant::now();
    let info = admin.share_info(&tokens.access, "bogus-key-w5").await;
    println!(
        "devicesTransInfo {} ms -> {:?}",
        started.elapsed().as_millis(),
        info.err().map(|error| error.to_string())
    );

    let marker = format!("{} W5", devices[0].display_name());
    let started = Instant::now();
    let renamed = admin
        .rename(&tokens.access, &uuid, &account, &marker)
        .await?;
    println!(
        "mdedit           {} ms -> code {}",
        started.elapsed().as_millis(),
        renamed["code"]
    );

    let started = Instant::now();
    let after = cloud.device_list(&tokens.access).await?;
    let seen = after
        .iter()
        .find(|row| row.uuid == uuid)
        .map(|row| row.display_name().to_string());
    println!(
        "msdlistCode      {} ms -> {:?}",
        started.elapsed().as_millis(),
        seen
    );

    let started = Instant::now();
    let back = admin
        .rename(&tokens.access, &uuid, &account, devices[0].display_name())
        .await?;
    println!(
        "mdedit restore   {} ms -> code {}",
        started.elapsed().as_millis(),
        back["code"]
    );

    let started = Instant::now();
    let share = admin
        .share_out(
            &tokens.access,
            std::slice::from_ref(&uuid),
            "nobody@example.invalid",
        )
        .await;
    println!(
        "devicesTransOut  {} ms -> {:?}",
        started.elapsed().as_millis(),
        share.err().map(|error| error.to_string())
    );
    Ok(())
}
