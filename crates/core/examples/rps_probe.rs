use std::time::Instant;

use proview_core::cloud::envelope::{EnvelopeClient, Identity};
use proview_core::cloud::status::{auth_code_of, StatusClient};
use proview_core::provider::{p2p, rps};

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let account = env("PROVIEW_EMAIL");
    let password = env("PROVIEW_PASSWORD");
    let serial = env("PROVIEW_DEVICE_SERIAL");
    let device_user = env("PROVIEW_DEVICE_USER");
    let device_pass = env("PROVIEW_DEVICE_PASS");
    if account.is_empty() || password.is_empty() || serial.is_empty() {
        return Err("set PROVIEW_EMAIL, PROVIEW_PASSWORD and PROVIEW_DEVICE_SERIAL".into());
    }
    let identity = Identity::resolve();
    if !identity.is_complete() {
        return Err(proview_core::cloud::MISSING_CLOUD_IDENTITY.into());
    }
    let client = EnvelopeClient::new(identity);

    let started = Instant::now();
    let tokens = client.login_envelope(&account, &password).await?;
    eprintln!("login    {:.2}s", started.elapsed().as_secs_f64());

    let started = Instant::now();
    let devices = client.device_list(&tokens.access).await?;
    let device = devices
        .iter()
        .find(|device| device.uuid == serial)
        .ok_or("the serial is not on this account")?;
    let rps_id = device
        .extra
        .get("rps")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    eprintln!(
        "devices  {:.2}s name={} rps={}",
        started.elapsed().as_secs_f64(),
        device.display_name(),
        rps_id
    );

    let started = Instant::now();
    let status = StatusClient::new("pub-status.secu100.net")
        .query(&serial, &auth_code_of(device))
        .await?;
    eprintln!(
        "status   {:.2}s online={} server={:?}:{:?}",
        started.elapsed().as_secs_f64(),
        status.online,
        status.server_ip,
        status.server_port
    );
    let (access_host, access_port) = status
        .access_server()
        .ok_or("no status cluster reported an access server")?;
    eprintln!("access   {access_host}:{access_port}");

    let session = rps::session_id();
    let token = match env("PROVIEW_CLIENT_TOKEN") {
        value if value.is_empty() => rps::client_token(),
        value => value,
    };
    eprintln!(
        "token    {} (persistent={})",
        token,
        !env("PROVIEW_CLIENT_TOKEN").is_empty()
    );
    let body = p2p::agent_request(
        &rps_id,
        &token,
        &serial,
        p2p::AGENT_SERVICE_MEDIA,
        &session,
        p2p::AGENT_STEP_MEDIA,
    );
    let started = Instant::now();
    let agent = rps::AccessClient::new()
        .assign(&access_host, access_port, &body)
        .await?;
    eprintln!(
        "agent    {:.2}s {}:{} session={}",
        started.elapsed().as_secs_f64(),
        agent.host,
        agent.port,
        session
    );

    let frame = p2p::handshake_frame(&rps_id, &token, &serial, &session);
    let started = Instant::now();
    let tunnel = rps::open(&agent, &frame).await?;
    eprintln!(
        "tunnel   {:.2}s error={} dest={} mtu={} accepted={}",
        started.elapsed().as_secs_f64(),
        tunnel.reply.error_num,
        tunnel.reply.dest_ip,
        tunnel.reply.mtu,
        tunnel.reply.accepted()
    );
    let mut connection = tunnel.into_connection();
    let started = Instant::now();
    let login = connection.login(&device_user, &device_pass).await?;
    eprintln!(
        "dvrip    {:.2}s session={} type={:?} channels={:?}",
        started.elapsed().as_secs_f64(),
        login.session,
        login.device_type,
        login.channels
    );
    let started = Instant::now();
    connection.claim_monitor(0, "Main").await?;
    if let Some(token) = std::env::var("PROVIEW_DEVICE_TOKEN")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        let sealed = match std::env::var("PROVIEW_DEVICE_MODULUS")
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            Some(modulus) => match proview_core::dvrip::crypto::communicate_key(
                modulus.trim(),
                "010001",
                &rps::session_key(),
            ) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("communicate key failed: {error}");
                    String::new()
                }
            },
            None => String::new(),
        };
        eprintln!("communicate key {} hex chars", sealed.len());
        match connection.login_token(token.trim(), &sealed).await {
            Ok(reply) => eprintln!("tokenlogin ret={:?} reply={}", reply.get("Ret"), reply),
            Err(error) => eprintln!("tokenlogin failed: {error:?}"),
        }
    }
    for index in 0..6 {
        let (header, payload) = match tokio::time::timeout(
            std::time::Duration::from_secs(6),
            connection.recv(),
        )
        .await
        {
            Ok(Ok(frame)) => frame,
            Ok(Err(error)) => {
                eprintln!("frame{index}   read failed: {error:?}");
                break;
            }
            Err(_) => {
                eprintln!("frame{index}   no more frames inside 6s");
                break;
            }
        };
        let preview: String = payload
            .iter()
            .take(60)
            .map(|byte| {
                if byte.is_ascii_graphic() {
                    *byte as char
                } else {
                    '.'
                }
            })
            .collect();
        eprintln!(
            "frame{}   {:.2}s id=0x{:04x} len={} {}",
            index,
            started.elapsed().as_secs_f64(),
            header.id,
            payload.len(),
            preview
        );
        if payload.is_empty() {
            break;
        }
    }
    let started = Instant::now();
    match connection.system_info().await {
        Ok(info) => eprintln!(
            "sysinfo  {:.2}s device={} software={}",
            started.elapsed().as_secs_f64(),
            info.get("DeviceInfo")
                .and_then(|v| v.get("DeviceName"))
                .and_then(|v| v.as_str())
                .unwrap_or("-"),
            info.get("DeviceInfo")
                .and_then(|v| v.get("SoftWareVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or("-")
        ),
        Err(error) => eprintln!(
            "sysinfo  {:.2}s skipped: {error:?}",
            started.elapsed().as_secs_f64()
        ),
    }
    Ok(())
}
