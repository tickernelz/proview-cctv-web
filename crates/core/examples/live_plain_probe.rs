use std::time::{Duration, Instant};

use proview_core::cloud::envelope::{EnvelopeClient, Identity};
use proview_core::cloud::status::{auth_code_of, StatusClient};
use proview_core::dvrip::crypto;
use proview_core::media::live::LiveDemux;
use proview_core::provider::{p2p, rps};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const HEADER_LEN: usize = 20;
const MONITOR_CHANNEL: u8 = 99;
const CLAIM_ID: u16 = 0x0585;
const START_ID: u16 = 0x0582;
const LOGIN_ID: u16 = 1000;
const MEDIA_ID: u16 = 0x0584;

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

struct Wire {
    stream: tokio::net::TcpStream,
    session: u32,
    sequence: u32,
}

impl Wire {
    fn new(stream: tokio::net::TcpStream) -> Self {
        Wire {
            stream,
            session: 0,
            sequence: 0,
        }
    }

    async fn send(&mut self, channel: u8, id: u16, payload: &[u8]) -> std::io::Result<()> {
        let mut frame = vec![0u8; HEADER_LEN];
        frame[0] = 0xff;
        frame[1] = 0x01;
        frame[4..8].copy_from_slice(&self.session.to_le_bytes());
        self.sequence = self.sequence.wrapping_add(1);
        frame[8..12].copy_from_slice(&self.sequence.to_le_bytes());
        frame[12] = channel;
        frame[14..16].copy_from_slice(&id.to_le_bytes());
        frame[16..20].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(payload);
        self.stream.write_all(&frame).await?;
        self.stream.flush().await
    }

    async fn send_value(
        &mut self,
        channel: u8,
        id: u16,
        value: &Value,
        nul: bool,
    ) -> std::io::Result<()> {
        let mut payload = serde_json::to_vec(value).map_err(std::io::Error::other)?;
        if nul {
            payload.push(0);
        } else {
            payload.push(b'\n');
            payload.push(0);
        }
        self.send(channel, id, &payload).await
    }

    async fn recv(&mut self, budget: Duration) -> Option<(u16, Vec<u8>)> {
        let mut header = [0u8; HEADER_LEN];
        tokio::time::timeout(budget, self.stream.read_exact(&mut header))
            .await
            .ok()?
            .ok()?;
        if header[0] != 0xff {
            return None;
        }
        let session = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        if session != 0 {
            self.session = session;
        }
        let id = u16::from_le_bytes([header[14], header[15]]);
        let length = u32::from_le_bytes([header[16], header[17], header[18], header[19]]);
        let mut body = vec![0u8; length as usize];
        if length > 0 {
            tokio::time::timeout(budget, self.stream.read_exact(&mut body))
                .await
                .ok()?
                .ok()?;
        }
        Some((id, body))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let account = env("PROVIEW_EMAIL");
    let password = env("PROVIEW_PASSWORD");
    let serial = env("PROVIEW_DEVICE_SERIAL");
    let device_user = env("PROVIEW_DEVICE_USER");
    let device_pass = env("PROVIEW_DEVICE_PASS");
    let stream_kind = env("LIVE_STREAM");
    let stream_kind = if stream_kind.is_empty() {
        "Main".to_string()
    } else {
        stream_kind
    };
    let seconds: u64 = env("LIVE_SECONDS").parse().unwrap_or(10);
    let out = env("LIVE_OUT");
    if account.is_empty() || password.is_empty() || serial.is_empty() {
        return Err("set PROVIEW_EMAIL, PROVIEW_PASSWORD and PROVIEW_DEVICE_SERIAL".into());
    }
    let identity = Identity::resolve();
    if !identity.is_complete() {
        return Err(proview_core::cloud::MISSING_CLOUD_IDENTITY.into());
    }
    let cloud = EnvelopeClient::new(identity);
    let tokens = cloud.login_envelope(&account, &password).await?;
    let devices = cloud.device_list(&tokens.access).await?;
    let device = devices
        .iter()
        .find(|entry| entry.uuid == serial)
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
    let (host, port) = status.access_server().ok_or("no access server")?;

    let session_id = rps::session_id();
    let client_token = rps::client_token();
    let request = p2p::agent_request(
        &rps_id,
        &client_token,
        &serial,
        p2p::AGENT_SERVICE_MEDIA,
        &session_id,
        p2p::AGENT_STEP_MEDIA,
    );
    let started = Instant::now();
    let agent = rps::AccessClient::new()
        .assign(&host, port, &request)
        .await?;
    let frame = p2p::handshake_frame(&rps_id, &client_token, &serial, &session_id);
    let tunnel = rps::open(&agent, &frame).await?;
    if !tunnel.reply.accepted() {
        return Err(format!("relay refused: {}", tunnel.reply.error_num).into());
    }
    eprintln!(
        "tunnel {:.2}s dest={}",
        started.elapsed().as_secs_f64(),
        tunnel.reply.dest_ip
    );
    let mut wire = Wire::new(tunnel.stream);

    wire.send_value(
        0,
        LOGIN_ID,
        &json!({
            "EncryptType": "MD5",
            "LoginType": "DVRIP-Web",
            "PassWord": crypto::sofia_hash(&device_pass),
            "UserName": device_user,
        }),
        false,
    )
    .await?;
    let login = wire
        .recv(Duration::from_secs(6))
        .await
        .ok_or("no login reply")?;
    let trimmed: Vec<u8> = login
        .1
        .iter()
        .copied()
        .rev()
        .skip_while(|byte| *byte == 0 || byte.is_ascii_whitespace())
        .collect::<Vec<u8>>()
        .into_iter()
        .rev()
        .collect();
    let login_json: Value = serde_json::from_slice(&trimmed).unwrap_or(Value::Null);
    let session = login_json
        .get("SessionID")
        .and_then(Value::as_str)
        .map(|text| text.trim_start_matches("0x").to_string())
        .and_then(|hex| u32::from_str_radix(&hex, 16).ok())
        .ok_or("the login reply carries no session")?;
    wire.session = session;
    eprintln!(
        "login {:.2}s ret={:?}",
        started.elapsed().as_secs_f64(),
        login_json.get("Ret")
    );

    wire.send_value(
        MONITOR_CHANNEL,
        CLAIM_ID,
        &json!({
            "Name": "OPMonitor",
            "OPMonitor": {
                "Action": "Claim",
                "Parameter": {
                    "Channel": 0,
                    "CombinMode": "CONNECT_ALL",
                    "StreamType": "Main",
                    "TransMode": "TCP",
                },
            },
            "DHParameter": { "RandomStrA": rps::random_lower(4) },
            "SessionID": format!("0x{session:08x}"),
        }),
        true,
    )
    .await?;
    if let Some((id, body)) = wire.recv(Duration::from_secs(6)).await {
        eprintln!(
            "claim reply id=0x{id:04x} {}",
            String::from_utf8_lossy(&body[..body.len().min(80)]).replace('\n', " ")
        );
    }

    let start = Instant::now();
    wire.send_value(
        0,
        START_ID,
        &proview_core::media::live::start_body(session, 0, &stream_kind),
        true,
    )
    .await?;
    let mut demux = LiveDemux::new();
    let mut elementary = Vec::new();
    let mut first_at: Option<f64> = None;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        let Some((id, body)) = wire.recv(Duration::from_secs(seconds)).await else {
            break;
        };
        if id != MEDIA_ID {
            continue;
        }
        if first_at.is_none() {
            first_at = Some(start.elapsed().as_secs_f64());
        }
        elementary.extend_from_slice(&demux.push(&body));
    }
    elementary.extend_from_slice(&demux.finish());
    let stats = demux.stats();
    eprintln!(
        "media first_at={first_at:?} keyframes={} deltas={} padding={} markers={} continuations={} dropped={} elementary={}",
        stats.keyframes,
        stats.deltas,
        stats.padding,
        stats.markers,
        stats.continuations,
        stats.dropped,
        elementary.len()
    );
    if !out.is_empty() {
        std::fs::write(&out, &elementary)?;
        eprintln!("wrote {out}");
    }
    let _ = stream_kind;
    Ok(())
}
