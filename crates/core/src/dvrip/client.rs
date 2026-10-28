use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::crypto::sofia_hash;
use super::frame::{FrameError, Header};
use super::msg;

#[derive(Debug)]
pub enum DvrError {
    Io(std::io::Error),
    Frame(FrameError),
    Malformed(String),
    Rejected { ret: i64, reason: &'static str },
    RealmChallenge { realm: String },
    NoReply,
}

impl std::fmt::Display for DvrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DvrError::Io(error) => write!(f, "io: {error}"),
            DvrError::Frame(error) => write!(f, "frame: {error:?}"),
            DvrError::Malformed(detail) => write!(f, "unexpected reply: {detail}"),
            DvrError::Rejected { ret, reason } => write!(f, "device returned ret {ret}: {reason}"),
            DvrError::RealmChallenge { realm } => write!(
                f,
                "device asked for a realm challenge (realm {realm}); this device family answers the sofia hash directly and no verified realm digest exists"
            ),
            DvrError::NoReply => write!(f, "device sent no reply"),
        }
    }
}

impl std::error::Error for DvrError {}

impl From<std::io::Error> for DvrError {
    fn from(error: std::io::Error) -> Self {
        DvrError::Io(error)
    }
}

impl From<FrameError> for DvrError {
    fn from(error: FrameError) -> Self {
        DvrError::Frame(error)
    }
}

pub fn describe_ret(ret: i64) -> &'static str {
    match ret {
        100 => "ok",
        101 => "unknown error",
        103 => "request not permitted",
        104 => "user already logged in",
        105 => "user is not logged in",
        106 => "username or password is incorrect",
        107 => "user lacks permission",
        118 => "ptz protocol not set",
        150 => "ok, device restart required",
        202 => "user is not logged in",
        203 => "password is incorrect",
        204 => "user is invalid",
        205 => "ip locked",
        206 => "user is in the blacklist",
        207 => "blacklisted",
        208 => "invalid input",
        502 => "command is invalid",
        503 => "talk is already open",
        504 => "talk is not open",
        605 => "feature not supported",
        607 => "configuration does not exist",
        _ => "device error",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginOk {
    pub session: u32,
    pub device_type: Option<String>,
    pub channels: Option<u32>,
    pub alive_interval: Option<u32>,
}

fn monitor_random() -> String {
    crate::provider::rps::random_lower(4)
}

#[cfg(test)]
mod monitor_tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn claim_monitor_frames_the_message_like_the_capture() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let accepted = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut header = [0u8; super::super::HEADER_LEN];
            socket.read_exact(&mut header).await.expect("header");
            let decoded = Header::decode(&header).expect("header decodes");
            let mut body = vec![0u8; decoded.length as usize];
            socket.read_exact(&mut body).await.expect("body");
            (decoded, body)
        });

        let stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("connect");
        let mut connection = Connection::from_stream(stream);
        connection
            .claim_monitor(0, "Main")
            .await
            .expect("claim sends");

        let (header, body) = accepted.await.expect("reader");
        assert_eq!(header.id, super::super::msg::MONITOR);
        assert_eq!(header.channel, super::super::msg::MONITOR_CHANNEL);
        assert!(!header.end, "the claim keeps the end bit clear");
        assert_eq!(body.last(), Some(&0), "the claim body ends with a NUL");
        let parsed: serde_json::Value =
            serde_json::from_slice(&body[..body.len() - 1]).expect("claim json");
        assert_eq!(parsed["Name"], "OPMonitor");
        assert_eq!(parsed["OPMonitor"]["Action"], "Claim");
        assert_eq!(parsed["OPMonitor"]["Parameter"]["TransMode"], "TCP");
        assert_eq!(parsed["SessionID"], format!("0x{:08x}", header.session));
    }
}

pub struct Connection {
    stream: TcpStream,
    session: u32,
    sequence: u32,
}

fn trim_reply(payload: &[u8]) -> &[u8] {
    let mut end = payload.len();
    while end > 0 && matches!(payload[end - 1], 0 | b'\n' | b'\r') {
        end -= 1;
    }
    &payload[..end]
}

fn parse_json(payload: &[u8]) -> Result<Value, DvrError> {
    serde_json::from_slice(trim_reply(payload)).map_err(|error| {
        DvrError::Malformed(format!(
            "{error}: {}",
            String::from_utf8_lossy(trim_reply(payload))
                .chars()
                .take(120)
                .collect::<String>()
        ))
    })
}

fn ret_of(reply: &Value) -> Result<i64, DvrError> {
    reply
        .get("Ret")
        .and_then(Value::as_i64)
        .ok_or_else(|| DvrError::Malformed("reply has no Ret field".to_string()))
}

pub fn require_ok(reply: &Value) -> Result<i64, DvrError> {
    let ret = ret_of(reply)?;
    if ret == 100 || ret == 150 {
        return Ok(ret);
    }
    Err(DvrError::Rejected {
        ret,
        reason: describe_ret(ret),
    })
}

fn parse_session_id(raw: &str) -> Option<u32> {
    let trimmed = raw.trim();
    match trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => trimmed.parse::<u32>().ok(),
    }
}

fn field(reply: &Value, names: &[&str]) -> Option<Value> {
    names.iter().find_map(|name| reply.get(*name).cloned())
}

impl Connection {
    pub async fn connect(address: &str) -> std::io::Result<Self> {
        let stream = TcpStream::connect(address).await?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            session: 0,
            sequence: 0,
        })
    }

    pub fn from_stream(stream: TcpStream) -> Self {
        stream.set_nodelay(true).ok();
        Self {
            stream,
            session: 0,
            sequence: 0,
        }
    }

    pub fn session(&self) -> u32 {
        self.session
    }

    fn next_sequence(&mut self) -> u32 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }

    async fn send_frame(&mut self, id: u16, body: &[u8]) -> std::io::Result<()> {
        self.send_frame_on(0, true, id, body).await
    }

    async fn send_frame_on(
        &mut self,
        channel: u8,
        end: bool,
        id: u16,
        body: &[u8],
    ) -> std::io::Result<()> {
        let sequence = self.next_sequence();
        let header = Header {
            session: self.session,
            sequence,
            channel,
            end,
            id,
            length: body.len() as u32,
        };
        self.stream.write_all(&header.encode()).await?;
        self.stream.write_all(body).await?;
        self.stream.flush().await
    }

    async fn send_json(&mut self, id: u16, body: &Value) -> Result<(), DvrError> {
        let mut payload =
            serde_json::to_vec(body).map_err(|error| DvrError::Malformed(error.to_string()))?;
        payload.push(b'\n');
        payload.push(0);
        self.send_frame(id, &payload).await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<(Header, Vec<u8>), FrameError> {
        let mut header_bytes = [0u8; super::HEADER_LEN];
        self.stream
            .read_exact(&mut header_bytes)
            .await
            .map_err(|_| FrameError::NotEnoughData)?;
        let header = Header::decode(&header_bytes)?;
        let mut body = vec![0u8; header.length as usize];
        if header.length > 0 {
            self.stream
                .read_exact(&mut body)
                .await
                .map_err(|_| FrameError::NotEnoughData)?;
        }
        if header.session != 0 {
            self.session = header.session;
        }
        Ok((header, body))
    }

    pub async fn claim_monitor(&mut self, channel: u8, stream: &str) -> Result<(), DvrError> {
        let body = serde_json::json!({
            "Name": "OPMonitor",
            "OPMonitor": {
                "Action": "Claim",
                "Parameter": {
                    "Channel": channel,
                    "CombinMode": "CONNECT_ALL",
                    "StreamType": stream,
                    "TransMode": "TCP",
                },
            },
            "DHParameter": { "RandomStrA": monitor_random() },
            "SessionID": format!("0x{:08x}", self.session),
        });
        let mut payload =
            serde_json::to_vec(&body).map_err(|error| DvrError::Malformed(error.to_string()))?;
        payload.push(0);
        self.send_frame_on(
            super::msg::MONITOR_CHANNEL,
            false,
            super::msg::MONITOR,
            &payload,
        )
        .await?;
        Ok(())
    }

    pub async fn request(&mut self, id: u16, body: Value) -> Result<Value, DvrError> {
        self.send_json(id, &body).await?;
        self.await_reply(8).await
    }

    async fn await_reply(&mut self, attempts: usize) -> Result<Value, DvrError> {
        for _ in 0..attempts {
            let (_, payload) = self.recv().await?;
            if let Ok(value) = parse_json(&payload) {
                if value.is_object() {
                    return Ok(value);
                }
            }
        }
        Err(DvrError::NoReply)
    }

    pub async fn login_body(&mut self, body: Value) -> Result<Value, DvrError> {
        let mut payload =
            serde_json::to_vec(&body).map_err(|error| DvrError::Malformed(error.to_string()))?;
        payload.push(0);
        self.send_frame_on(0, true, super::msg::LOGIN, &payload)
            .await?;
        let (_, reply) = self.recv().await?;
        parse_json(&reply)
    }

    pub async fn login_token(
        &mut self,
        token: &str,
        communicate_key: &str,
    ) -> Result<Value, DvrError> {
        let body = json!({
            "EncryptType": "TOKEN",
            "LoginType": "DVRIP-FutureHome",
            "UserName": token,
            "PassWord": "",
            "CommunicateKey": communicate_key,
        });
        let mut payload =
            serde_json::to_vec(&body).map_err(|error| DvrError::Malformed(error.to_string()))?;
        payload.push(0);
        self.send_frame_on(
            super::msg::MONITOR_CHANNEL,
            false,
            super::msg::LOGIN,
            &payload,
        )
        .await?;
        let (_, reply) = self.recv().await?;
        parse_json(&reply)
    }

    pub async fn send_plain(
        &mut self,
        channel: u8,
        id: u16,
        body: &serde_json::Value,
    ) -> Result<(), DvrError> {
        let mut payload =
            serde_json::to_vec(body).map_err(|error| DvrError::Malformed(error.to_string()))?;
        payload.push(0);
        self.send_frame_on(channel, false, id, &payload).await?;
        Ok(())
    }

    pub async fn send_encrypted(
        &mut self,
        cipher: &crate::media::dhav::MediaCipher,
        channel: u8,
        id: u16,
        body: &serde_json::Value,
    ) -> Result<(), DvrError> {
        let plain =
            serde_json::to_vec(body).map_err(|error| DvrError::Malformed(error.to_string()))?;
        let mut payload = cipher.encrypt_body(&plain).into_bytes();
        payload.push(0);
        self.send_frame_on(channel, false, id, &payload).await?;
        Ok(())
    }

    pub async fn start_monitor(
        &mut self,
        cipher: &crate::media::dhav::MediaCipher,
        channel: u8,
        stream: &str,
        vekey1: &str,
    ) -> Result<(), DvrError> {
        let body = json!({
            "Name": "OPMonitor",
            "OPMonitor": {
                "Action": "Start",
                "Parameter": {
                    "Channel": channel,
                    "CombinMode": "NONE",
                    "StreamType": stream,
                    "TransMode": "TCP",
                    "VEKEY1": vekey1,
                },
            },
            "SessionID": format!("0x{:08x}", self.session),
        });
        let plain =
            serde_json::to_vec(&body).map_err(|error| DvrError::Malformed(error.to_string()))?;
        let mut payload = cipher.encrypt_body(&plain).into_bytes();
        payload.push(0);
        self.send_frame_on(0, false, super::msg::MONITOR_START, &payload)
            .await?;
        Ok(())
    }

    pub async fn login(&mut self, user: &str, password: &str) -> Result<LoginOk, DvrError> {
        let body = json!({
            "EncryptType": "MD5",
            "LoginType": "DVRIP-Web",
            "PassWord": sofia_hash(password),
            "UserName": user,
        });
        let reply = self.request(msg::LOGIN, body).await?;
        let ret = ret_of(&reply)?;
        if ret == 200 {
            let realm = field(&reply, &["Realm"])
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default();
            return Err(DvrError::RealmChallenge { realm });
        }
        require_ok(&reply)?;
        if let Some(session) = field(&reply, &["SessionID"])
            .and_then(|value| value.as_str().map(parse_session_id))
            .flatten()
        {
            self.session = session;
        }
        Ok(LoginOk {
            session: self.session,
            device_type: field(&reply, &["DeviceType ", "DeviceType"])
                .and_then(|value| value.as_str().map(str::to_string)),
            channels: field(&reply, &["ChannelNum"])
                .and_then(|value| value.as_u64())
                .map(|value| value as u32),
            alive_interval: field(&reply, &["AliveInterval"])
                .and_then(|value| value.as_u64())
                .map(|value| value as u32),
        })
    }

    pub async fn keep_alive(&mut self) -> Result<(), DvrError> {
        let session = format!("0x{:08X}", self.session);
        let reply = self
            .request(
                msg::KEEP_ALIVE,
                json!({ "Name": "KeepAlive", "SessionID": session }),
            )
            .await?;
        require_ok(&reply).map(|_| ())
    }

    pub async fn logout(&mut self) -> Result<(), DvrError> {
        let session = format!("0x{:08X}", self.session);
        let reply = self
            .request(msg::LOGOUT, json!({ "Name": "", "SessionID": session }))
            .await?;
        self.session = 0;
        require_ok(&reply).map(|_| ())
    }

    pub async fn logout_within(&mut self, window: std::time::Duration) -> Result<(), DvrError> {
        match tokio::time::timeout(window, self.logout()).await {
            Ok(result) => result,
            Err(_) => {
                self.session = 0;
                Err(DvrError::NoReply)
            }
        }
    }

    pub async fn snapshot(&mut self, channel: u8) -> Result<Vec<u8>, DvrError> {
        let session = format!("0x{:08X}", self.session);
        self.send_json(
            msg::SNAP,
            &json!({
                "Name": "OPSNAP",
                "SessionID": session,
                "OPSNAP": { "Channel": channel },
            }),
        )
        .await?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline {
            let (_, payload) = self.recv().await?;
            if payload.len() > 3 && payload[0] == 0xff && payload[1] == 0xd8 && payload[2] == 0xff {
                return Ok(payload);
            }
            if let Ok(reply) = parse_json(&payload) {
                if let Ok(ret) = ret_of(&reply) {
                    if ret != 100 {
                        return Err(DvrError::Rejected {
                            ret,
                            reason: describe_ret(ret),
                        });
                    }
                }
            }
        }
        Err(DvrError::NoReply)
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{connect, json_reply, serve};
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn session_ids_parse_from_hex_or_decimal() {
        assert_eq!(parse_session_id("0x00a1b2c3"), Some(0x00a1_b2c3));
        assert_eq!(parse_session_id("0X0000002A"), Some(42));
        assert_eq!(parse_session_id("42"), Some(42));
        assert_eq!(parse_session_id("not-a-number"), None);
    }

    #[tokio::test]
    async fn login_sends_the_sofia_hash_and_accepts_the_devices_field_names() {
        let (address, handle) = serve(vec![json_reply(json!({
            "Ret": 100,
            "SessionID": "0x00000017",
            "DeviceType ": "IPC",
            "ChannelNum": 1,
            "AliveInterval": 30,
            "AdminToken": "ignored",
        }))])
        .await;
        let mut connection = connect(&address).await;

        let ok = connection.login("admin", "hunter2").await.unwrap();

        assert_eq!(ok.session, 0x17);
        assert_eq!(ok.device_type.as_deref(), Some("IPC"));
        assert_eq!(ok.channels, Some(1));
        assert_eq!(ok.alive_interval, Some(30));
        let sent: Value =
            serde_json::from_str(handle.await.unwrap()[0].trim_end_matches('\0')).unwrap();
        assert_eq!(sent["UserName"], "admin");
        assert_eq!(sent["LoginType"], "DVRIP-Web");
        assert_eq!(sent["EncryptType"], "MD5");
        assert_eq!(sent["PassWord"], sofia_hash("hunter2"));
        assert_ne!(sent["PassWord"], "hunter2");
    }

    #[tokio::test]
    async fn rejected_login_reports_the_device_reason() {
        let (address, _handle) = serve(vec![json_reply(json!({ "Ret": 203 }))]).await;
        let mut connection = connect(&address).await;
        match connection.login("admin", "nope").await {
            Err(DvrError::Rejected { ret, reason }) => {
                assert_eq!(ret, 203);
                assert_eq!(reason, "password is incorrect");
            }
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn realm_challenge_is_reported_instead_of_guessed_again() {
        let (address, handle) =
            serve(vec![json_reply(json!({ "Ret": 200, "Realm": "a1b2c3d4" }))]).await;
        let mut connection = connect(&address).await;
        match connection.login("admin", "admin").await {
            Err(DvrError::RealmChallenge { realm }) => assert_eq!(realm, "a1b2c3d4"),
            other => panic!("expected realm challenge, got {other:?}"),
        }
        assert_eq!(handle.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn trailing_nul_and_newline_in_replies_are_ignored() {
        let (address, _handle) = serve(vec![json_reply(json!({
            "Ret": 100,
            "SessionID": "7",
        }))])
        .await;
        let mut connection = connect(&address).await;
        let ok = connection.login("admin", "admin").await.unwrap();
        assert_eq!(ok.session, 7);
    }

    async fn serve_pushes(replies: Vec<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut header_bytes = [0u8; super::super::HEADER_LEN];
            socket.read_exact(&mut header_bytes).await.unwrap();
            let header = Header::decode(&header_bytes).unwrap();
            let mut body = vec![0u8; header.length as usize];
            socket.read_exact(&mut body).await.unwrap();
            for reply in replies {
                let response = Header {
                    session: 0x3355_7799,
                    sequence: header.sequence,
                    channel: 0,
                    end: true,
                    id: header.id.wrapping_add(1),
                    length: reply.len() as u32,
                };
                socket.write_all(&response.encode()).await.unwrap();
                socket.write_all(&reply).await.unwrap();
            }
        });
        format!("{addr}")
    }

    #[tokio::test]
    async fn snapshot_returns_the_jpeg_payload_after_a_text_reply() {
        let jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10];
        let address = serve_pushes(vec![
            json_reply(json!({ "Ret": 100, "Name": "OPSNAP" })),
            jpeg.clone(),
        ])
        .await;
        let mut connection = connect(&address).await;
        let picture = connection.snapshot(0).await.unwrap();
        assert_eq!(picture, jpeg);
    }

    #[tokio::test]
    async fn snapshot_failure_code_is_reported_instead_of_waiting() {
        let address = serve_pushes(vec![json_reply(json!({ "Ret": 605, "Name": "OPSNAP" }))]).await;
        let mut connection = connect(&address).await;
        match connection.snapshot(0).await {
            Err(DvrError::Rejected { ret, reason }) => {
                assert_eq!(ret, 605);
                assert_eq!(reason, "feature not supported");
            }
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bare_numeric_error_codes_are_mapped_to_their_meaning() {
        assert_eq!(describe_ret(203), "password is incorrect");
        assert_eq!(describe_ret(605), "feature not supported");
        assert_eq!(describe_ret(607), "configuration does not exist");
        assert!(describe_ret(100).contains("ok"));
    }
}
