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
    if std::env::var("PROVIEW_LOGIN_DUMP").is_ok() {
        let reply = connection.login_reply(&device_user, &device_pass).await?;
        let mut shape = Vec::new();
        if let Some(object) = reply.as_object() {
            for (key, value) in object {
                let text = match value.as_str() {
                    Some(text) => format!("str{}", text.chars().count()),
                    None => value.to_string(),
                };
                shape.push(format!("{key}={text}"));
            }
        }
        eprintln!("loginraw {}", shape.join(" "));
        if let Ok(target) = std::env::var("PROVIEW_LOGIN_TOKEN_DUMP") {
            if let Some(token) = reply
                .get("AdminToken")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                std::fs::write(&target, token)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))?;
                }
                eprintln!("loginraw token written");
            } else {
                eprintln!("loginraw the reply carries no AdminToken");
            }
        }
        return Ok(());
    }
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
        let session_key = rps::session_key();
        let sealed = match std::env::var("PROVIEW_DEVICE_MODULUS")
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            Some(modulus) => match proview_core::dvrip::crypto::communicate_key(
                modulus.trim(),
                "010001",
                &session_key,
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
        if std::env::var("PROVIEW_PLAIN_QUERY").is_ok() {
            let session_text = format!("0x{:08x}", connection.session());
            let body = serde_json::json!({ "Name": "SystemInfo", "SessionID": session_text });
            match connection.send_plain(0, 0x03fc, &body).await {
                Ok(()) => eprintln!("plainq   sent"),
                Err(error) => eprintln!("plainq failed: {error:?}"),
            }
            match tokio::time::timeout(std::time::Duration::from_secs(5), connection.recv()).await {
                Ok(Ok((h, p))) => eprintln!(
                    "plainq reply id=0x{:04x} {}",
                    h.id,
                    String::from_utf8_lossy(&p[..p.len().min(80)])
                ),
                other => eprintln!("plainq reply: {other:?}"),
            }
            let mut parameter = serde_json::json!({
                "Channel": 0,
                "CombinMode": "NONE",
                "StreamType": "Main",
                "TransMode": "TCP",
            });
            if std::env::var("PROVIEW_VEKEY").is_ok() {
                parameter["VEKEY1"] = serde_json::Value::String(rps::random_lower(60));
            }
            let start = serde_json::json!({
                "Name": "OPMonitor",
                "OPMonitor": { "Action": "Start", "Parameter": parameter },
                "SessionID": format!("0x{:08x}", connection.session()),
            });
            match connection.send_plain(0, 0x0582, &start).await {
                Ok(()) => eprintln!(
                    "plainstart sent (vekey={})",
                    std::env::var("PROVIEW_VEKEY").is_ok()
                ),
                Err(error) => eprintln!("plainstart failed: {error:?}"),
            }
            let mut dump = std::fs::File::create("/tmp/live_bodies.bin").ok();
            let mut written = 0usize;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
            while std::time::Instant::now() < deadline {
                match tokio::time::timeout(std::time::Duration::from_secs(3), connection.recv())
                    .await
                {
                    Ok(Ok((header, payload))) => {
                        if header.id == 0x0584 {
                            if let Some(file) = dump.as_mut() {
                                use std::io::Write;
                                let _ = file.write_all(&(payload.len() as u32).to_le_bytes());
                                let _ = file.write_all(&payload);
                                written += payload.len();
                            }
                        }
                    }
                    _ => break,
                }
            }
            eprintln!("media    {written} bytes on 0x0584 in 8 s");
        }
        if let Ok(file) = std::env::var("PROVIEW_LOGIN_JSON") {
            let raw = std::fs::read_to_string(&file)?;
            let body: serde_json::Value = serde_json::from_str(raw.trim())?;
            let control = proview_core::media::dhav::MediaCipher::channel();
            match connection.send_encrypted(&control, 99, 0x03e8, &body).await {
                Ok(()) => eprintln!(
                    "verbatim login sent ({} fields)",
                    body.as_object().map(|o| o.len()).unwrap_or(0)
                ),
                Err(error) => eprintln!("verbatim login failed: {error:?}"),
            }
            match tokio::time::timeout(std::time::Duration::from_secs(6), connection.recv()).await {
                Ok(Ok((header, payload))) => eprintln!(
                    "verbatim reply id=0x{:04x} {}",
                    header.id,
                    String::from_utf8_lossy(&payload[..payload.len().min(120)])
                ),
                other => eprintln!("verbatim reply: {other:?}"),
            }
        }
        let encrypted_login = std::env::var("PROVIEW_ENCRYPTED_LOGIN").is_ok();
        if encrypted_login {
            let cipher = proview_core::media::dhav::MediaCipher::from_text(&session_key)
                .ok_or("the session key is not sixteen bytes")?;
            let body = serde_json::json!({
                "EncryptType": "TOKEN",
                "LoginType": "DVRIP-FutureHome",
                "UserName": token.trim(),
                "PassWord": "",
                "CommunicateKey": sealed,
            });
            match connection.send_encrypted(&cipher, 99, 0x03e8, &body).await {
                Ok(()) => eprintln!("enclogin sent"),
                Err(error) => eprintln!("enclogin failed: {error:?}"),
            }
            match tokio::time::timeout(std::time::Duration::from_secs(6), connection.recv()).await {
                Ok(Ok((header, payload))) => eprintln!(
                    "enclogin reply id=0x{:04x} {}",
                    header.id,
                    String::from_utf8_lossy(&payload[..payload.len().min(80)])
                ),
                other => eprintln!("enclogin reply: {other:?}"),
            }
        } else {
            match connection.login_token(token.trim(), &sealed).await {
                Ok(reply) => eprintln!("tokenlogin ret={:?} reply={}", reply.get("Ret"), reply),
                Err(error) => eprintln!("tokenlogin failed: {error:?}"),
            }
        }
        let cipher = proview_core::media::dhav::MediaCipher::from_text(&session_key)
            .ok_or("the session key is not sixteen bytes")?;
        let vekey1 = rps::random_lower(60);
        let session_text = format!("0x{:08x}", connection.session());
        for (id, name) in [
            (0x03fcu16, "SystemInfo"),
            (0x0550, "SystemFunction"),
            (0x0412, "fVideo.AudioSupportType"),
        ] {
            let body = serde_json::json!({ "Name": name, "SessionID": session_text });
            match connection.send_encrypted(&cipher, 0, id, &body).await {
                Ok(()) => eprintln!("query    sent {}", name),
                Err(error) => eprintln!("query {} failed: {error:?}", name),
            }
        }
        match connection.start_monitor(&cipher, 0, "Main", &vekey1).await {
            Ok(()) => eprintln!("start    sent vekey1={} chars", vekey1.len()),
            Err(error) => eprintln!("start failed: {error:?}"),
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
