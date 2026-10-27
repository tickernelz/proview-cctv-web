use std::error::Error;
use std::path::PathBuf;

type BoxError = Box<dyn Error + Send + Sync>;
use std::time::{Duration, Instant};

use proview_core::cloud::envelope::{EnvelopeClient, Identity};
use proview_core::cloud::status::{auth_code_of, StatusClient};
use proview_core::dvrip::client::Connection;
use proview_core::dvrip::settings::{EncodePatch, EncodeState, Stream};
use proview_core::provider::{p2p, rps};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

struct Relay {
    tunnel: rps::Tunnel,
    serial: String,
    agent: String,
}

async fn relay() -> Result<Relay, BoxError> {
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
    let tokens = client.login_envelope(&account, &password).await?;
    let devices = client.device_list(&tokens.access).await?;
    let wanted = env("PROVIEW_DEVICE_SERIAL");
    let device = devices
        .iter()
        .find(|device| !wanted.is_empty() && device.uuid == wanted)
        .or_else(|| devices.first())
        .ok_or("no device is on this account")?;
    let serial = device.uuid.clone();
    let rps_id = device
        .extra
        .get("rps")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    if rps_id.is_empty() {
        return Err("the device reports no rps id, so no relay path exists".into());
    }
    let status = StatusClient::new("pub-status.secu100.net")
        .query(&serial, &auth_code_of(device))
        .await?;
    if !status.online {
        return Err("the device is offline in the vendor cloud".into());
    }
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
    let agent = format!("{}:{}", agent.host, agent.port);
    Ok(Relay {
        tunnel,
        serial,
        agent,
    })
}

async fn connection() -> Result<(Connection, String, String), BoxError> {
    let relay = relay().await?;
    let mut connection = relay.tunnel.into_connection();
    connection
        .login(&env("PROVIEW_DEVICE_USER"), &env("PROVIEW_DEVICE_PASS"))
        .await?;
    Ok((connection, relay.serial, relay.agent))
}

fn print_state(label: &str, state: &EncodeState) {
    for (name, format) in [("main", &state.main), ("extra", &state.extra)] {
        println!(
            "{label} {name}: codec={:?} resolution={:?} fps={:?} bitrate={:?} gop={:?} quality={:?} control={:?}",
            format.compression, format.resolution, format.fps, format.bitrate, format.gop, format.quality, format.bitrate_control
        );
    }
}

fn backup_dir() -> PathBuf {
    PathBuf::from(
        std::env::args()
            .nth(2)
            .unwrap_or_else(|| "settings-backups".to_string()),
    )
}

fn stream() -> Stream {
    match env("PROVIEW_STREAM").as_str() {
        "extra" => Stream::Extra,
        _ => Stream::Main,
    }
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "read".to_string());
    match mode.as_str() {
        "read" => {
            let started = Instant::now();
            let (mut connection, serial, agent) = connection().await?;
            eprintln!("login {:.2}s serial={serial} agent={agent}", started.elapsed().as_secs_f64());
            let started = Instant::now();
            let state = connection.encode_state().await?;
            eprintln!("encode {:.2}s", started.elapsed().as_secs_f64());
            print_state("read", &state);
            println!("encode_json: {}", serde_json::to_string(&state)?);
            let started = Instant::now();
            let storage = connection.storage_state().await?;
            eprintln!("storage {:.2}s", started.elapsed().as_secs_f64());
            println!("storage_json: {}", serde_json::to_string(&storage)?);
            let started = Instant::now();
            let network = connection.network_state().await?;
            eprintln!("network {:.2}s", started.elapsed().as_secs_f64());
            println!("network_json: {}", serde_json::to_string(&network)?);
            let _ = connection.logout().await;
        }
        "write" => {
            let dir = backup_dir();
            let stream = stream();
            let patch = EncodePatch {
                fps: env("PROVIEW_FPS").parse::<i64>().ok(),
                bitrate: env("PROVIEW_BITRATE").parse::<i64>().ok(),
                gop: env("PROVIEW_GOP").parse::<i64>().ok(),
                resolution: None,
            };
            let (mut connection, serial, agent) = connection().await?;
            eprintln!("tunnel serial={serial} agent={agent}");
            let before = connection.encode_state().await?;
            print_state("before", &before);
            let report = connection.write_encode(stream, &patch, &dir).await?;
            println!("report_json: {}", serde_json::to_string(&report)?);
            let middle = connection.encode_state().await?;
            print_state("written", &middle);
            let format = match stream {
                Stream::Main => &before.main,
                Stream::Extra => &before.extra,
            };
            let original = EncodePatch {
                fps: format.fps,
                bitrate: format.bitrate,
                gop: format.gop,
                resolution: format.resolution.clone(),
            };
            let restore = connection.write_encode(stream, &original, &dir).await?;
            println!("restore_json: {}", serde_json::to_string(&restore)?);
            let after = connection.encode_state().await?;
            print_state("restored", &after);
            let _ = connection.logout().await;
        }
        "negate" => {
            let dir = backup_dir();
            let (mut connection, serial, agent) = connection().await?;
            eprintln!("tunnel serial={serial} agent={agent}");
            let before = connection.encode_state().await?;
            print_state("before", &before);
            let patch = EncodePatch {
                fps: Some(59),
                bitrate: Some(16384),
                gop: Some(200),
                resolution: None,
            };
            let report = connection.write_encode(Stream::Main, &patch, &dir).await?;
            println!("report_json: {}", serde_json::to_string(&report)?);
            let after = connection.encode_state().await?;
            print_state("after", &after);
            let _ = connection.logout().await;
        }
        "bridge" => {
            let listen = std::env::args()
                .nth(2)
                .unwrap_or_else(|| "127.0.0.1:34567".to_string());
            let listener = TcpListener::bind(&listen).await?;
            eprintln!("bridge listening on {listen}");
            loop {
                let (mut client, peer) = listener.accept().await?;
                eprintln!("bridge accept {peer}");
                tokio::spawn(async move {
                    let opened = tokio::time::timeout(Duration::from_secs(25), relay()).await;
                    let ready: Result<(TcpStream, String), String> = match opened {
                        Ok(Ok(relay)) => Ok((
                            relay.tunnel.stream,
                            format!("serial={} agent={}", relay.serial, relay.agent),
                        )),
                        Ok(Err(error)) => Err(format!("bridge tunnel failed: {error}")),
                        Err(_) => Err("bridge tunnel timed out".to_string()),
                    };
                    match ready {
                        Ok((mut stream, label)) => {
                            eprintln!("bridge tunnel {label}");
                            let _ = tokio::io::copy_bidirectional(&mut client, &mut stream).await;
                        }
                        Err(detail) => eprintln!("{detail}"),
                    }
                    let _ = client.shutdown().await;
                });
            }
        }
        other => return Err(format!("unknown mode {other}: read, write, negate or bridge").into()),
    }
    Ok(())
}
