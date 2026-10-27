use std::io::{Read, Write};
use std::net::{TcpStream as StdTcpStream, ToSocketAddrs};
use std::time::Duration;

use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
use rand::Rng;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use super::p2p::{self, AgentServer, RelayReply, WireError};

pub const ACCESS_PORT: u16 = 6510;
const ASSIGN_TIMEOUT: Duration = Duration::from_secs(10);
const TUNNEL_TIMEOUT: Duration = Duration::from_secs(10);
const REPLY_LIMIT: usize = 4096;
const CLIENT_TOKEN_LEN: usize = 25;
const SESSION_ID_LEN: usize = 32;

#[derive(Debug, Error)]
pub enum RpsError {
    #[error("the access server is unreachable: {0}")]
    Unreachable(String),
    #[error("the access server answered HTTP {0}")]
    Status(String),
    #[error("the wire is not what was captured: {0}")]
    Wire(#[from] WireError),
    #[error("no answer inside {0:?}")]
    Timeout(Duration),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn client_token() -> String {
    format!("c_{}", random_lower(CLIENT_TOKEN_LEN))
}

pub fn session_id() -> String {
    random_lower(SESSION_ID_LEN)
}

pub fn session_key() -> String {
    random_lower(16)
}

pub fn random_lower(len: usize) -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

pub struct AccessClient;

impl AccessClient {
    pub fn new() -> Self {
        Self
    }

    pub async fn assign(
        &self,
        host: &str,
        port: u16,
        request: &str,
    ) -> Result<AgentServer, RpsError> {
        let target = host.to_string();
        let body = request.to_string();
        let exchange = tokio::task::spawn_blocking(move || post(&target, port, &body));
        let reply = timeout(ASSIGN_TIMEOUT + Duration::from_secs(1), exchange)
            .await
            .map_err(|_| RpsError::Timeout(ASSIGN_TIMEOUT))?
            .map_err(|error| RpsError::Unreachable(error.to_string()))??;
        Ok(p2p::parse_agent_body(reply.as_bytes())?)
    }
}

impl Default for AccessClient {
    fn default() -> Self {
        Self::new()
    }
}

fn post(host: &str, port: u16, body: &str) -> Result<String, RpsError> {
    let mut builder = SslConnector::builder(SslMethod::tls())
        .map_err(|error| RpsError::Unreachable(error.to_string()))?;
    builder.set_verify(SslVerifyMode::NONE);
    let connector = builder.build();

    let address = (host, port)
        .to_socket_addrs()
        .map_err(|error| RpsError::Unreachable(error.to_string()))?
        .next()
        .ok_or_else(|| RpsError::Unreachable(format!("{host}:{port} does not resolve")))?;
    let stream = StdTcpStream::connect_timeout(&address, ASSIGN_TIMEOUT)
        .map_err(|error| RpsError::Unreachable(error.to_string()))?;
    stream.set_read_timeout(Some(ASSIGN_TIMEOUT)).ok();
    stream.set_write_timeout(Some(ASSIGN_TIMEOUT)).ok();
    let mut stream = connector
        .connect(host, stream)
        .map_err(|error| RpsError::Unreachable(error.to_string()))?;

    let request = format!(
        "POST / HTTP/1.1\r\nHost: {host}\r\nContent-Type: text/html\r\nAccept: */*\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| RpsError::Unreachable(error.to_string()))?;
    let mut reply = Vec::new();
    stream
        .read_to_end(&mut reply)
        .map_err(|error| RpsError::Unreachable(error.to_string()))?;
    let text = String::from_utf8_lossy(&reply).into_owned();
    let split = text
        .find("\r\n\r\n")
        .ok_or_else(|| RpsError::Unreachable("no HTTP headers in the access reply".into()))?;
    let code = text[..split]
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();
    if code != "200" {
        return Err(RpsError::Status(code));
    }
    Ok(text[split + 4..].to_string())
}

pub struct Tunnel {
    pub stream: TcpStream,
    pub reply: RelayReply,
}

impl Tunnel {
    pub fn into_connection(self) -> crate::dvrip::client::Connection {
        crate::dvrip::client::Connection::from_stream(self.stream)
    }
}

pub async fn open(agent: &AgentServer, frame: &[u8]) -> Result<Tunnel, RpsError> {
    timeout(TUNNEL_TIMEOUT, open_inner(agent, frame))
        .await
        .map_err(|_| RpsError::Timeout(TUNNEL_TIMEOUT))?
}

async fn open_inner(agent: &AgentServer, frame: &[u8]) -> Result<Tunnel, RpsError> {
    let mut stream = timeout(
        TUNNEL_TIMEOUT,
        TcpStream::connect((agent.host.as_str(), agent.port)),
    )
    .await
    .map_err(|_| RpsError::Timeout(TUNNEL_TIMEOUT))??;
    stream.set_nodelay(true).ok();
    stream.write_all(frame).await?;

    let mut buffer = Vec::with_capacity(REPLY_LIMIT);
    let mut chunk = [0u8; 512];
    loop {
        if let Ok(reply) = p2p::parse_relay_reply(&buffer) {
            return Ok(Tunnel { stream, reply });
        }
        if buffer.len() >= REPLY_LIMIT {
            return Err(RpsError::Wire(WireError::TooShort));
        }
        let read = timeout(TUNNEL_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| RpsError::Timeout(TUNNEL_TIMEOUT))??;
        if read == 0 {
            return Err(RpsError::Wire(WireError::TooShort));
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}
