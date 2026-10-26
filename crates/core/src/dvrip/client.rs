use serde::de::DeserializeOwned;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::crypto::{login_hash, realm_hash, realm_hash_with_legacy_secret};
use super::frame::{FrameError, Header};
use super::msg;

#[derive(Debug)]
pub enum LoginError {
    Io(std::io::Error),
    Frame(FrameError),
    Json(serde_json::Error),
    Rejected { ret: i64, message: Option<String> },
    NoRealmAfterChallenge,
}

impl std::fmt::Display for LoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoginError::Io(e) => write!(f, "io: {e}"),
            LoginError::Frame(e) => write!(f, "frame: {e:?}"),
            LoginError::Json(e) => write!(f, "json: {e}"),
            LoginError::Rejected { ret, message } => match message {
                Some(m) => write!(f, "device rejected login (ret {ret}): {m}"),
                None => write!(f, "device rejected login (ret {ret})"),
            },
            LoginError::NoRealmAfterChallenge => {
                write!(f, "device asked for realm login without a Realm field")
            }
        }
    }
}

impl std::error::Error for LoginError {}

#[derive(Debug)]
pub struct LoginOk {
    pub session: u32,
    pub device_type: Option<String>,
}

#[derive(Deserialize)]
struct LoginResponse {
    #[serde(rename = "Ret")]
    ret: i64,
    #[serde(rename = "SessionID", default)]
    session_id: Option<String>,
    #[serde(rename = "Realm", default)]
    realm: Option<String>,
    #[serde(rename = "DeviceType", default)]
    device_type: Option<String>,
    #[serde(rename = "Error", default)]
    error: Option<String>,
}

fn parse_session_id(raw: &str) -> Option<u32> {
    let trimmed = raw.trim();
    let without_prefix = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"));
    match without_prefix {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => trimmed.parse::<u32>().ok(),
    }
}

pub struct Connection {
    stream: TcpStream,
    session: u32,
    sequence: u32,
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

    fn next_sequence(&mut self) -> u32 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }

    pub async fn send(&mut self, id: u16, channel: u8, body: &[u8]) -> std::io::Result<u32> {
        let sequence = self.next_sequence();
        let header = Header {
            session: self.session,
            sequence,
            channel,
            end: true,
            id,
            length: body.len() as u32,
        };
        self.stream.write_all(&header.encode()).await?;
        self.stream.write_all(body).await?;
        self.stream.flush().await?;
        Ok(sequence)
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
        self.session = header.session;
        Ok((header, body))
    }

    pub async fn login_variant(
        &mut self,
        user: &str,
        password: &str,
        variant: &str,
    ) -> Result<LoginOk, LoginError> {
        let candidate = match variant {
            "md5" => login_hash(user, password),
            "md5md5" => crate::dvrip::crypto::md5_hex(&login_hash(user, password)),
            "plain" => password.to_string(),
            other => {
                return Err(LoginError::Rejected {
                    ret: -1,
                    message: Some(format!("unknown variant {other}")),
                })
            }
        };
        let response = self.login_attempt(user, &candidate).await?;
        if response.ret == 100 {
            return Ok(LoginOk {
                session: self.session,
                device_type: response.device_type,
            });
        }
        if response.ret == 200 {
            let realm = response
                .realm
                .clone()
                .ok_or(LoginError::NoRealmAfterChallenge)?;
            let retry = match variant {
                "md5" => realm_hash(user, &realm, password),
                "md5md5" => realm_hash_with_legacy_secret(user, &realm, &candidate),
                _ => realm_hash(user, &realm, password),
            };
            let after = self.login_attempt(user, &retry).await?;
            if after.ret == 100 {
                return Ok(LoginOk {
                    session: self.session,
                    device_type: after.device_type,
                });
            }
            return Err(LoginError::Rejected {
                ret: after.ret,
                message: after.error,
            });
        }
        Err(LoginError::Rejected {
            ret: response.ret,
            message: response.error,
        })
    }

    pub async fn login(&mut self, user: &str, password: &str) -> Result<LoginOk, LoginError> {
        let probe = self.login_attempt(user, "").await?;
        if probe.ret == 100 {
            return Ok(LoginOk {
                session: self.session,
                device_type: probe.device_type,
            });
        }
        let first = if probe.ret == 200 {
            probe
        } else {
            let direct = self
                .login_attempt(user, &login_hash(user, password))
                .await?;
            if direct.ret == 100 {
                return Ok(LoginOk {
                    session: self.session,
                    device_type: direct.device_type,
                });
            }
            return Err(LoginError::Rejected {
                ret: direct.ret,
                message: direct.error,
            });
        };

        if first.ret != 200 {
            return Err(LoginError::Rejected {
                ret: first.ret,
                message: first.error,
            });
        }

        let realm = first
            .realm
            .clone()
            .ok_or(LoginError::NoRealmAfterChallenge)?;
        let secret = login_hash(user, password);
        let candidates = [
            realm_hash(user, &realm, password),
            realm_hash_with_legacy_secret(user, &realm, &secret),
        ];

        let mut last = first;
        for candidate in candidates {
            let response = self.login_attempt(user, &candidate).await?;
            if response.ret == 100 {
                return Ok(LoginOk {
                    session: self.session,
                    device_type: response.device_type,
                });
            }
            last = response;
        }
        Err(LoginError::Rejected {
            ret: last.ret,
            message: last.error,
        })
    }

    async fn login_attempt(
        &mut self,
        user: &str,
        password_hash: &str,
    ) -> Result<LoginResponse, LoginError> {
        let body = serde_json::json!({
            "EncryptType": "MD5",
            "LoginType": "DVRIP-Web",
            "PassWord": password_hash,
            "UserName": user,
        });
        self.send(msg::LOGIN_REQUEST, 0, body.to_string().as_bytes())
            .await
            .map_err(LoginError::Io)?;
        let (_, payload) = self.recv().await.map_err(LoginError::Frame)?;
        let mut deserializer = serde_json::Deserializer::from_slice(&payload);
        let parsed = LoginResponse::deserialize(&mut deserializer).map_err(LoginError::Json)?;
        if let Some(raw) = parsed.session_id.as_deref() {
            if let Some(session) = parse_session_id(raw) {
                self.session = session;
            }
        }
        Ok(parsed)
    }

    pub async fn start_monitor(&mut self, channel: u8, sub_stream: bool) -> std::io::Result<u32> {
        let body = serde_json::json!({
            "Name": "OPMonitor",
            "OPMonitor": {
                "Action": "Start",
                "Parameter": {
                    "Channel": channel,
                    "CombinMode": "CONNECT",
                    "StreamType": if sub_stream { "Extra1" } else { "Main" },
                    "TransMode": "TCP",
                }
            }
        });
        self.send(msg::MONITOR_START, channel, body.to_string().as_bytes())
            .await
    }

    pub async fn stop_monitor(&mut self, channel: u8) -> std::io::Result<u32> {
        let body = serde_json::json!({
            "Name": "OPMonitor",
            "OPMonitor": { "Action": "Stop", "Parameter": { "Channel": channel } }
        });
        self.send(msg::MONITOR_STOP, channel, body.to_string().as_bytes())
            .await
    }

    pub async fn read_frame(&mut self) -> Result<(Header, Vec<u8>), FrameError> {
        self.recv().await
    }

    pub async fn logout(&mut self) -> std::io::Result<()> {
        self.send(msg::LOGOUT, 0, b"{}").await?;
        self.session = 0;
        Ok(())
    }

    pub fn session(&self) -> u32 {
        self.session
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn spawn_reply(replies: Vec<serde_json::Value>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            for reply in replies {
                let mut header_bytes = [0u8; super::super::HEADER_LEN];
                if socket.read_exact(&mut header_bytes).await.is_err() {
                    return;
                }
                let header = Header::decode(&header_bytes).unwrap();
                let mut body = vec![0u8; header.length as usize];
                socket.read_exact(&mut body).await.unwrap();

                let payload = reply.to_string();
                let response = Header {
                    session: 0x1122_3344,
                    sequence: header.sequence,
                    channel: 0,
                    end: true,
                    id: header.id,
                    length: payload.len() as u32,
                };
                socket.write_all(&response.encode()).await.unwrap();
                socket.write_all(payload.as_bytes()).await.unwrap();
            }
        });
        format!("{addr}")
    }

    #[test]
    fn session_id_is_read_from_hex_or_decimal() {
        assert_eq!(parse_session_id("0x00a1b2c3"), Some(0x00a1_b2c3));
        assert_eq!(parse_session_id("0X0000002A"), Some(42));
        assert_eq!(parse_session_id("42"), Some(42));
        assert_eq!(parse_session_id("not-a-number"), None);
    }

    #[tokio::test]
    async fn plain_login_succeeds_when_the_device_answers_100() {
        let addr = spawn_reply(vec![serde_json::json!({"Ret": 100, "DeviceType": "IPC"})]).await;
        let mut connection = Connection::connect(&addr).await.unwrap();
        let ok = connection.login("admin", "admin").await.unwrap();
        assert_eq!(ok.session, 0x1122_3344);
        assert_eq!(ok.device_type.as_deref(), Some("IPC"));
    }

    #[tokio::test]
    async fn realm_challenge_is_answered_with_the_realm_hash() {
        let realm = "a1b2c3d4";
        let expected = realm_hash("admin", realm, "admin");
        let replies = vec![
            serde_json::json!({"Ret": 200, "Realm": realm, "EncryptType": "MD5"}),
            serde_json::json!({"Ret": 100}),
        ];
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut seen = Vec::new();
            for reply in replies {
                let mut header_bytes = [0u8; super::super::HEADER_LEN];
                socket.read_exact(&mut header_bytes).await.unwrap();
                let header = Header::decode(&header_bytes).unwrap();
                let mut body = vec![0u8; header.length as usize];
                socket.read_exact(&mut body).await.unwrap();
                seen.push(String::from_utf8(body).unwrap());

                let payload = reply.to_string();
                let response = Header {
                    session: 0x5566_7788,
                    sequence: header.sequence,
                    channel: 0,
                    end: true,
                    id: header.id,
                    length: payload.len() as u32,
                };
                socket.write_all(&response.encode()).await.unwrap();
                socket.write_all(payload.as_bytes()).await.unwrap();
            }
            let second: serde_json::Value = serde_json::from_str(&seen[1]).unwrap();
            assert_eq!(second["PassWord"], expected);
        });

        let mut connection = Connection::connect(&format!("{addr}")).await.unwrap();
        let ok = connection.login("admin", "admin").await.unwrap();
        assert_eq!(ok.session, 0x5566_7788);
    }

    #[tokio::test]
    async fn monitor_start_sends_the_documented_shape_and_media_is_returned_raw() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();

            let mut header_bytes = [0u8; super::super::HEADER_LEN];
            socket.read_exact(&mut header_bytes).await.unwrap();
            let header = Header::decode(&header_bytes).unwrap();
            let mut body = vec![0u8; header.length as usize];
            socket.read_exact(&mut body).await.unwrap();
            let request: serde_json::Value = serde_json::from_slice(&body).unwrap();

            let media = [0u8, 0, 0, 1, 0x65, 0x88, 0x84];
            let response = Header {
                session: 7,
                sequence: header.sequence,
                channel: 3,
                end: true,
                id: header.id,
                length: media.len() as u32,
            };
            socket.write_all(&response.encode()).await.unwrap();
            socket.write_all(&media).await.unwrap();
            request
        });

        let mut connection = Connection::connect(&format!("{addr}")).await.unwrap();
        connection.start_monitor(3, false).await.unwrap();
        let (header, payload) = connection.read_frame().await.unwrap();
        let request = handle.await.unwrap();

        assert_eq!(request["Name"], "OPMonitor");
        assert_eq!(request["OPMonitor"]["Action"], "Start");
        assert_eq!(request["OPMonitor"]["Parameter"]["Channel"], 3);
        assert_eq!(request["OPMonitor"]["Parameter"]["StreamType"], "Main");
        assert_eq!(header.id, super::super::msg::MONITOR_START);
        assert_eq!(header.channel, 3);
        assert_eq!(payload, vec![0u8, 0, 0, 1, 0x65, 0x88, 0x84]);
    }

    #[tokio::test]
    async fn sub_stream_selects_the_extra_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut header_bytes = [0u8; super::super::HEADER_LEN];
            socket.read_exact(&mut header_bytes).await.unwrap();
            let header = Header::decode(&header_bytes).unwrap();
            let mut body = vec![0u8; header.length as usize];
            socket.read_exact(&mut body).await.unwrap();
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()
        });
        let mut connection = Connection::connect(&format!("{addr}")).await.unwrap();
        connection.start_monitor(1, true).await.unwrap();
        let request = handle.await.unwrap();
        assert_eq!(request["OPMonitor"]["Parameter"]["StreamType"], "Extra1");
    }

    #[tokio::test]
    async fn rejected_login_is_reported_with_the_device_message() {
        let addr = spawn_reply(vec![
            serde_json::json!({"Ret": 401, "Error": "bad password"}),
        ])
        .await;
        let mut connection = Connection::connect(&addr).await.unwrap();
        match connection.login("admin", "nope").await {
            Err(LoginError::Rejected { ret, message }) => {
                assert_eq!(ret, 401);
                assert_eq!(message.as_deref(), Some("bad password"));
            }
            other => panic!("expected rejection, got {other:?}"),
        }
    }
}
