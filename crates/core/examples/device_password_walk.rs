use proview_core::cloud::envelope::{EnvelopeClient, Identity};
use proview_core::cloud::status::{auth_code_of, StatusClient, STATUS_HOST};
use proview_core::dvrip::client::Connection;
use proview_core::provider::p2p;
use proview_core::provider::rps::{self as rps_client, AccessClient};

struct Cloud {
    client: EnvelopeClient,
    access: String,
    device: String,
}

async fn tunnel(cloud: &Cloud) -> Result<Connection, Box<dyn std::error::Error>> {
    let devices = cloud.client.device_list(&cloud.access).await?;
    let entry = devices
        .iter()
        .find(|row| row.uuid == cloud.device)
        .ok_or("device not in the account list")?;
    let rps = entry
        .extra
        .get("rps")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let status = StatusClient::new(STATUS_HOST);
    let state = status.query(&entry.uuid, &auth_code_of(entry)).await?;
    let (host, port) = state.access_server().ok_or("no access server reported")?;
    for attempt in 1..=3 {
        let client_token = rps_client::client_token();
        let session_id = rps_client::session_id();
        let request = p2p::agent_request(
            &rps,
            &client_token,
            &entry.uuid,
            p2p::AGENT_SERVICE_MEDIA,
            &session_id,
            p2p::AGENT_STEP_MEDIA,
        );
        if let Ok(agent) = AccessClient::new().assign(&host, port, &request).await {
            let frame = p2p::handshake_frame(&rps, &client_token, &entry.uuid, &session_id);
            if let Ok(link) = rps_client::open(&agent, &frame).await {
                println!(
                    "  tunnel {attempt} accepted dest={} mtu={}",
                    link.reply.dest_ip, link.reply.mtu
                );
                return Ok(link.into_connection());
            }
        }
    }
    Err("no tunnel".into())
}

async fn session(cloud: &Cloud, user: &str, password: &str) -> Result<Connection, String> {
    let mut connection = tunnel(cloud).await.map_err(|error| error.to_string())?;
    match connection.login(user, password).await {
        Ok(ok) => {
            println!(
                "  login ok session=0x{:08X} type={:?}",
                ok.session, ok.device_type
            );
            Ok(connection)
        }
        Err(error) => Err(error.to_string()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let account = std::env::var("PROVIEW_EMAIL")?;
    let account_pass = std::env::var("PROVIEW_PASSWORD")?;
    let user = std::env::var("PROVIEW_DEVICE_USER").unwrap_or_else(|_| "admin".to_string());
    let current = std::env::var("PROVIEW_DEVICE_PASS")?;
    let device = std::env::var("PROVIEW_DEVICE_SN")?;
    let next = std::env::var("PROVIEW_DEVICE_NEW_PASS")?;

    let client = EnvelopeClient::new(Identity::resolve());
    let tokens = client.login_envelope(&account, &account_pass).await?;
    let cloud = Cloud {
        client,
        access: tokens.access,
        device,
    };
    println!("account login ok");

    if std::env::var("PROVIEW_DEVICE_USERS").as_deref() == Ok("1") {
        let mut connection = session(&cloud, &user, &current).await?;
        let users = connection.config_get("System.ExUserMap").await?;
        let list = users
            .get("System.ExUserMap")
            .and_then(|value| value.get("User"))
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        println!("users={}", list.len());
        for entry in list {
            let name = entry
                .get("Name")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let groups = entry
                .get("Group")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let keys: Vec<&str> = entry
                .as_object()
                .map(|map| map.keys().map(String::as_str).collect())
                .unwrap_or_default();
            println!("  name={name} group={groups} keys={keys:?}");
        }
        return Ok(());
    }

    let variant = std::env::var("PROVIEW_DEVICE_VARIANT").unwrap_or_else(|_| "upper".to_string());
    let encode = |text: &str| -> String {
        match variant.as_str() {
            "plain" => text.to_string(),
            "lower" => proview_core::dvrip::crypto::md5_hex(text),
            _ => proview_core::dvrip::crypto::md5_hex(text).to_uppercase(),
        }
    };
    let body = |session: u32, from: &str, to: &str| {
        serde_json::json!({
            "EncryptType": "MD5",
            "PassWord": encode(from),
            "NewPassWord": encode(to),
            "SessionID": format!("0x{session:08X}"),
            "UserName": user,
        })
    };
    println!("phase 1: change with the current password (variant {variant})");
    let mut connection = session(&cloud, &user, &current).await?;
    let ret = connection
        .config_set(
            "ModifyPassword",
            body(connection.session(), &current, &next),
        )
        .await?;
    println!("  modify_password ret={ret}");
    drop(connection);

    println!("phase 2: fresh login with the new password");
    let changed = session(&cloud, &user, &next).await;
    match &changed {
        Ok(_) => println!("  new password works on a fresh session"),
        Err(error) => println!("  new password refused: {error}"),
    }

    println!("phase 3: revert to the original password");
    let revert = match changed {
        Ok(mut connection) => {
            connection
                .config_set(
                    "ModifyPassword",
                    body(connection.session(), &next, &current),
                )
                .await
        }
        Err(_) => {
            let mut connection = session(&cloud, &user, &current).await?;
            connection
                .config_set(
                    "ModifyPassword",
                    body(connection.session(), &current, &next),
                )
                .await
        }
    };
    println!("  revert ret={revert:?}");

    println!("phase 4: fresh login with the original password");
    match session(&cloud, &user, &current).await {
        Ok(_) => println!("  original password restored"),
        Err(error) => println!("  original password refused: {error}"),
    }
    Ok(())
}
