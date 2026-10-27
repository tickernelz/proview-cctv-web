use proview_core::cloud::envelope::{EnvelopeClient, Identity};
use proview_core::cloud::status::{auth_code_of, StatusClient};
use proview_core::provider::{p2p, rps};
use serde_json::Value;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

async fn tunnel() -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    let account = env("PROVIEW_EMAIL");
    let password = env("PROVIEW_PASSWORD");
    let serial = env("PROVIEW_DEVICE_SERIAL");
    if account.is_empty() || password.is_empty() || serial.is_empty() {
        return Err("set PROVIEW_EMAIL, PROVIEW_PASSWORD and PROVIEW_DEVICE_SERIAL".into());
    }
    let identity = Identity::resolve();
    if !identity.is_complete() {
        return Err(proview_core::cloud::MISSING_CLOUD_IDENTITY.into());
    }
    let client = EnvelopeClient::new(identity);
    let tokens = client.login_envelope(&account, &password).await?;
    let devices = client.device_list(&tokens.access).await?;
    let device = devices
        .iter()
        .find(|device| device.uuid == serial)
        .ok_or("the serial is not on this account")?;
    let rps_id = device
        .extra
        .get("rps")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let status = StatusClient::new("pub-status.secu100.net")
        .query(&serial, &auth_code_of(device))
        .await?;
    let (access_host, access_port) = status
        .access_server()
        .ok_or("no status cluster reported an access server")?;
    let session = rps::session_id();
    let token = rps::client_token();
    let body = p2p::agent_request(
        &rps_id,
        &token,
        &serial,
        p2p::AGENT_SERVICE_MEDIA,
        &session,
        p2p::AGENT_STEP_MEDIA,
    );
    let agent = rps::AccessClient::new()
        .assign(&access_host, access_port, &body)
        .await?;
    let frame = p2p::handshake_frame(&rps_id, &token, &serial, &session);
    let tunnel = rps::open(&agent, &frame).await?;
    eprintln!(
        "relay {}:{} dest {} mtu {}",
        agent.host, agent.port, tunnel.reply.dest_ip, tunnel.reply.mtu
    );
    Ok(tunnel.stream)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind = {
        let raw = env("PROVIEW_BIND");
        if raw.is_empty() {
            "127.0.0.1:34567".to_string()
        } else {
            raw
        }
    };
    let listener = TcpListener::bind(&bind).await?;
    eprintln!("bridge listening on {bind}");
    loop {
        let (mut client, peer) = listener.accept().await?;
        eprintln!("bridge accepted {peer}");
        tokio::spawn(async move {
            match tunnel().await {
                Ok(mut relay) => {
                    if let Err(error) = copy_bidirectional(&mut client, &mut relay).await {
                        eprintln!("bridge {peer} ended: {error}");
                    } else {
                        eprintln!("bridge {peer} closed");
                    }
                }
                Err(error) => eprintln!("bridge {peer} could not open the relay: {error}"),
            }
        });
    }
}
