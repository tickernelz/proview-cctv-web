use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Deserialize;

pub const HANDSHAKE_TRAILER: &[u8; 4] = b"XXEE";
pub const RELAY_PORT: u16 = 6611;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    TooShort,
    BadText,
    Unsupported(&'static str),
}

impl std::fmt::Display for WireError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::TooShort => write!(formatter, "the reply is shorter than its own header"),
            WireError::BadText => {
                write!(formatter, "the reply carries text that is not base64 json")
            }
            WireError::Unsupported(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for WireError {}

pub fn handshake(rps: &str, client: &str, serial: &str, token: &str) -> String {
    let plain = format!("{rps}:{client}:{serial}:{token}");
    STANDARD.encode(plain)
}

pub fn handshake_frame(rps: &str, client: &str, serial: &str, token: &str) -> Vec<u8> {
    let mut frame = handshake(rps, client, serial, token).into_bytes();
    frame.extend_from_slice(HANDSHAKE_TRAILER);
    frame
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RelayReply {
    #[serde(rename = "ErrorNum")]
    pub error_num: String,
    #[serde(rename = "DestIP")]
    pub dest_ip: String,
    #[serde(rename = "MTU")]
    pub mtu: String,
}

impl RelayReply {
    pub fn accepted(&self) -> bool {
        self.error_num == "200"
    }
}

pub fn parse_relay_reply(bytes: &[u8]) -> Result<RelayReply, WireError> {
    let trailer = bytes
        .windows(HANDSHAKE_TRAILER.len())
        .position(|window| window == HANDSHAKE_TRAILER)
        .ok_or(WireError::Unsupported(
            "relay reply without the XXEE trailer",
        ))?;
    let text = std::str::from_utf8(&bytes[..trailer]).map_err(|_| WireError::BadText)?;
    let json = STANDARD
        .decode(text.trim())
        .map_err(|_| WireError::BadText)?;
    let plain = String::from_utf8(json).map_err(|_| WireError::BadText)?;
    serde_json::from_str(&plain).map_err(|_| WireError::BadText)
}

pub const AGENT_SERVICE_MEDIA: &str = "RpsAV";
pub const AGENT_STEP_MEDIA: &str = "8";
pub const AGENT_DEST_PORT: &str = "34567";

pub fn agent_request(
    authcode: &str,
    client_token: &str,
    serial: &str,
    service: &str,
    session_id: &str,
    step: &str,
) -> String {
    serde_json::json!({
        "AgentProtocol": {
            "Body": {
                "Authcode": authcode,
                "ClientToken": client_token,
                "DestPort": AGENT_DEST_PORT,
                "Encrypt": "1",
                "ProtocolMode": "tcp",
                "SerialNumber": serial,
                "ServiceType": service,
                "SessionId": session_id,
                "Step": step,
            },
            "Header": {
                "MessageType": "MSG_CLI_NEED_CON_REQ",
                "Version": "1.0",
            },
        }
    })
    .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentServer {
    pub host: String,
    pub port: u16,
}

#[derive(Deserialize)]
struct AgentReply {
    #[serde(rename = "AgentProtocol")]
    protocol: AgentReplyBody,
}

#[derive(Deserialize)]
struct AgentReplyBody {
    #[serde(rename = "Body")]
    body: AgentEndpoint,
    #[serde(rename = "Header")]
    header: AgentHeader,
}

#[derive(Deserialize)]
struct AgentEndpoint {
    #[serde(rename = "AgentServerIp")]
    host: String,
    #[serde(rename = "AgentServerPort")]
    port: String,
}

#[derive(Deserialize)]
struct AgentHeader {
    #[serde(rename = "ErrorNum")]
    error_num: String,
}

pub fn parse_agent_body(bytes: &[u8]) -> Result<AgentServer, WireError> {
    let body = match bytes.windows(4).position(|window| window == b"\r\n\r\n") {
        Some(split) => &bytes[split + 4..],
        None => bytes,
    };
    let reply: AgentReply = serde_json::from_slice(body).map_err(|_| WireError::BadText)?;
    if reply.protocol.header.error_num != "200" {
        return Err(WireError::Unsupported(
            "the agent reply refused the session",
        ));
    }
    let port = reply
        .protocol
        .body
        .port
        .trim()
        .parse()
        .map_err(|_| WireError::BadText)?;
    Ok(AgentServer {
        host: reply.protocol.body.host,
        port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERIAL: &str = "0123456789abcdef0123";

    #[test]
    fn builds_the_captured_handshake_text() {
        let text = handshake(
            "aaaaaaaa-1a1b397d2",
            "c_6kjkfw65cvzl39rjrno8yc0j",
            SERIAL,
            "cy7cimmp5",
        );
        let decoded = STANDARD.decode(text).unwrap();
        assert_eq!(
            String::from_utf8(decoded).unwrap(),
            format!("aaaaaaaa-1a1b397d2:c_6kjkfw65cvzl39rjrno8yc0j:{SERIAL}:cy7cimmp5")
        );
    }

    #[test]
    fn hands_over_the_media_session_id_in_the_handshake_frame() {
        let frame = handshake_frame(
            "aaaaaaaa-1a1b397d2",
            "c_rpftjva4cpy4aphg09y083ya",
            SERIAL,
            "y8ekrmh5ud5wtaoak22y5o0vxz5l1qzg",
        );
        assert_eq!(&frame[frame.len() - 4..], HANDSHAKE_TRAILER);
        let text = std::str::from_utf8(&frame[..frame.len() - 4]).expect("base64 text");
        let plain = STANDARD.decode(text).expect("base64 decodes");
        assert_eq!(
            String::from_utf8(plain).unwrap(),
            format!(
                "aaaaaaaa-1a1b397d2:c_rpftjva4cpy4aphg09y083ya:{SERIAL}:y8ekrmh5ud5wtaoak22y5o0vxz5l1qzg"
            )
        );
    }

    #[test]
    fn parses_the_captured_relay_reply() {
        let reply = parse_relay_reply(
            b"eyJFcnJvck51bSI6IjIwMCIsIkRlc3RJUCI6IjEwMy44NS4xMjIuNjYiLCJNVFUiOiIxNDAwIn0=XXEE",
        )
        .expect("relay reply");
        assert!(reply.accepted());
        assert_eq!(reply.dest_ip, "103.85.122.66");
        assert_eq!(reply.mtu, "1400");
    }

    #[test]
    fn rejects_a_reply_without_the_trailer() {
        assert!(parse_relay_reply(b"eyJ9").is_err());
    }

    #[test]
    fn builds_the_captured_agent_request() {
        let built = agent_request(
            "aaaaaaaa-1a1b2f27d",
            "c_1yy07itliajqv91mz6249eet",
            SERIAL,
            AGENT_SERVICE_MEDIA,
            "04yebeay8cpxqwob4ze5u7y0iev07ffo",
            AGENT_STEP_MEDIA,
        );
        let parsed: serde_json::Value = serde_json::from_str(&built).expect("agent request json");
        let body = &parsed["AgentProtocol"]["Body"];
        assert_eq!(body["ServiceType"], "RpsAV");
        assert_eq!(body["Step"], "8");
        assert_eq!(body["DestPort"], "34567");
        assert_eq!(body["ProtocolMode"], "tcp");
        assert_eq!(body["Encrypt"], "1");
        assert_eq!(body["SerialNumber"], SERIAL);
        assert_eq!(
            parsed["AgentProtocol"]["Header"]["MessageType"],
            "MSG_CLI_NEED_CON_REQ"
        );
        let keys: Vec<&String> = body.as_object().expect("body object").keys().collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "the json keys keep the captured order");
    }

    const CAPTURED_REPLY_BODY: &str = r#"{"AgentProtocol":{"Body":{"AgentServerIp":"162.128.64.50","AgentServerLanIp":"10.210.0.2","AgentServerDNS":"162.128.64.50","AgentServerPort":"6611","ProtocolMode":"tcp"},"Header":{"CSeq":"3","Version":"1.0","MessageType":"MSG_CLI_NEED_CON_RSP","ErrorNum":"200","ErrorString":"Success OK"}}}"#;

    #[test]
    fn parses_the_captured_agent_reply() {
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 290\r\n\r\n{}",
            CAPTURED_REPLY_BODY
        );
        let server = parse_agent_body(reply.as_bytes()).expect("agent reply");
        assert_eq!(server.host, "162.128.64.50");
        assert_eq!(server.port, 6611);
    }

    #[test]
    fn refuses_an_agent_reply_that_is_not_200() {
        let refused = CAPTURED_REPLY_BODY.replace("\"200\"", "\"400\"");
        let reply = format!("HTTP/1.1 200 OK\r\n\r\n{refused}");
        assert_eq!(
            parse_agent_body(reply.as_bytes()),
            Err(WireError::Unsupported(
                "the agent reply refused the session"
            ))
        );
    }
}
