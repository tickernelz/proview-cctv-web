use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Deserialize;

pub const DISCOVERY_MAGIC: [u8; 2] = [0x12, 0x20];
pub const OP_SERVER_LIST_REQ: [u16; 1] = [0xf005];
pub const OP_SERVER_LIST_RSP: u16 = 0xf105;
pub const HANDSAHKE_TRAILER: &[u8; 4] = b"XXEE";
pub const RELAY_PORT: u16 = 6611;
pub const DISCOVERY_PORT: u16 = 8765;
pub const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relay {
    pub host: String,
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    TooShort,
    BadMagic,
    BadOpcode,
    BadText,
    Unsupported(&'static str),
}

impl std::fmt::Display for WireError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::TooShort => write!(formatter, "the reply is shorter than its own header"),
            WireError::BadMagic => write!(formatter, "the reply does not start with 12 20"),
            WireError::BadOpcode => write!(formatter, "the reply carries an unknown opcode"),
            WireError::BadText => write!(
                formatter,
                "the reply carries text that is not utf-8 base64 json"
            ),
            WireError::Unsupported(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for WireError {}

pub fn discovery_request() -> [u8; 4] {
    [
        DISCOVERY_MAGIC[0],
        DISCOVERY_MAGIC[1],
        (OP_SERVER_LIST_REQ[0] >> 8) as u8,
        OP_SERVER_LIST_REQ[0] as u8,
    ]
}

pub fn parse_relay_list(bytes: &[u8]) -> Result<Vec<Relay>, WireError> {
    if bytes.len() < 8 {
        return Err(WireError::TooShort);
    }
    if bytes[0..2] != DISCOVERY_MAGIC {
        return Err(WireError::BadMagic);
    }
    let opcode = u16::from_be_bytes([bytes[2], bytes[3]]);
    if opcode != OP_SERVER_LIST_RSP {
        return Err(WireError::BadOpcode);
    }
    let count = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let mut relays = Vec::with_capacity(count);
    let mut cursor = 8usize;
    for _ in 0..count {
        let end = bytes[cursor..]
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(WireError::TooShort)?
            + cursor;
        let host = std::str::from_utf8(&bytes[cursor..end])
            .map_err(|_| WireError::BadText)?
            .to_string();
        cursor = end + 1;
        while (cursor - 8) % 4 != 0 {
            cursor += 1;
        }
        if bytes.len() < cursor + 4 {
            return Err(WireError::TooShort);
        }
        let weight = u32::from_le_bytes([
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ]);
        cursor += 4;
        relays.push(Relay { host, weight });
    }
    Ok(relays)
}

pub fn handshake(rps: &str, client: &str, serial: &str, token: &str) -> String {
    let plain = format!("{rps}:{client}:{serial}:{token}");
    STANDARD.encode(plain)
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

    pub fn mtu_bytes(&self) -> u16 {
        self.mtu.trim().parse().unwrap_or(1400)
    }
}

pub fn parse_relay_reply(bytes: &[u8]) -> Result<RelayReply, WireError> {
    let trailer = bytes
        .windows(HANDSAHKE_TRAILER.len())
        .position(|window| window == HANDSAHKE_TRAILER)
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

#[cfg(test)]
mod tests {
    use super::*;

    const DISCOVERY_REPLY: &[u8] = &[
        0x12, 0x20, 0xf1, 0x05, 0x03, 0x00, 0x00, 0x00, b'1', b'5', b'9', b'.', b'1', b'3', b'8',
        b'.', b'1', b'0', b'0', b'.', b'1', b'3', b'3', 0x00, 0x55, 0x00, 0x00, 0x00, b'1', b'5',
        b'9', b'.', b'1', b'3', b'8', b'.', b'1', b'0', b'1', b'.', b'1', b'2', b'3', 0x00, 0x45,
        0x00, 0x00, 0x00, b'1', b'1', b'0', b'.', b'2', b'3', b'9', b'.', b'7', b'1', b'.', b'2',
        b'2', b'4', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn asks_for_the_relay_set_with_the_captured_frame() {
        assert_eq!(discovery_request(), [0x12, 0x20, 0xf0, 0x05]);
    }

    #[test]
    fn parses_the_captured_relay_list() {
        let relays = parse_relay_list(DISCOVERY_REPLY).expect("relay list");
        assert_eq!(relays.len(), 3);
        assert_eq!(relays[0].host, "159.138.100.133");
        assert_eq!(relays[0].weight, 85);
        assert_eq!(relays[2].host, "110.239.71.224");
    }

    const LIVE_DISCOVERY_REPLY: &[u8] = &[
        0x12, 0x20, 0xf1, 0x05, 0x03, 0x00, 0x00, 0x00, b'1', b'7', b'2', b'.', b'1', b'0', b'5',
        b'.', b'2', b'1', b'9', b'.', b'1', b'9', b'7', 0x00, 0x35, 0x00, 0x00, 0x00, b'1', b'7',
        b'2', b'.', b'1', b'0', b'5', b'.', b'4', b'2', b'.', b'4', b'2', 0x00, 0x00, 0x00, 0x45,
        0x00, 0x00, 0x00, b'1', b'1', b'0', b'.', b'2', b'3', b'9', b'.', b'7', b'1', b'.', b'2',
        b'2', b'4', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn parses_the_live_list_where_entries_are_word_aligned() {
        let relays = parse_relay_list(LIVE_DISCOVERY_REPLY).expect("relay list");
        assert_eq!(relays.len(), 3);
        assert_eq!(relays[0].host, "172.105.219.197");
        assert_eq!(relays[0].weight, 53);
        assert_eq!(relays[1].host, "172.105.42.42");
        assert_eq!(relays[1].weight, 69);
        assert_eq!(relays[2].host, "110.239.71.224");
        assert_eq!(relays[2].weight, 0);
    }

    #[test]
    fn rejects_a_frame_with_the_wrong_magic() {
        let mut broken = DISCOVERY_REPLY.to_vec();
        broken[0] = 0x00;
        assert_eq!(parse_relay_list(&broken), Err(WireError::BadMagic));
    }

    #[test]
    fn builds_the_captured_handshake_text() {
        let text = handshake(
            "aaaaaaaa-1a1b397d2",
            "c_6kjkfw65cvzl39rjrno8yc0j",
            "7f92970ddf5282743hrc",
            "cy7cimmp5",
        );
        let decoded = STANDARD.decode(text).unwrap();
        assert_eq!(
            String::from_utf8(decoded).unwrap(),
            "aaaaaaaa-1a1b397d2:c_6kjkfw65cvzl39rjrno8yc0j:7f92970ddf5282743hrc:cy7cimmp5"
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
        assert_eq!(reply.mtu_bytes(), 1400);
    }

    #[test]
    fn rejects_a_reply_without_the_trailer() {
        assert!(parse_relay_reply(b"eyJ9").is_err());
    }
}
