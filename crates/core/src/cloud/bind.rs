use std::sync::atomic::{AtomicU32, Ordering};

use serde::Deserialize;

use super::client::CloudError;
use super::envelope::{
    open, seal, signed_request, url_encode, EnvelopeClient, Identity, SIGNED_VERSION,
};
use super::types::{Envelope, CODE_OK, CODE_OK_LEGACY};

pub const ADD_DEVICE_OK: i32 = CODE_OK;
pub const ADD_DEVICE_ALREADY: i32 = 4101;
pub const ADD_DEVICE_USER_FAILED: i32 = 4105;
pub const QR_CODEC_KEY: &str = "DecInfoEncode";
pub const QR_CODEC_MARK: char = 'z';
pub const QR_OFFSET_LIMIT: u32 = 744;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddDevice {
    pub serial: String,
    pub username: String,
    pub password: String,
    pub account_id: String,
    pub nickname: String,
    pub kind: u32,
    pub ip: String,
    pub port: u32,
}

impl AddDevice {
    pub fn form(&self) -> String {
        let mut out = String::new();
        out.push_str("uuid=");
        out.push_str(&url_encode(&self.serial));
        out.push_str("&username=");
        out.push_str(&url_encode(&self.username));
        out.push_str("&password=");
        out.push_str(&url_encode(&self.password));
        out.push_str("&userId=");
        out.push_str(&url_encode(&self.account_id));
        out.push_str("&nickname=");
        out.push_str(&url_encode(&self.nickname));
        out.push_str(&format!("&type={}", self.kind));
        out.push_str("&ip=");
        out.push_str(&url_encode(&self.ip));
        out.push_str(&format!("&port={}", self.port));
        out
    }
}

#[derive(Debug)]
pub enum BindFailure {
    Vendor { code: i32, message: String },
    Cloud(CloudError),
}

impl BindFailure {
    pub fn expired(&self) -> bool {
        matches!(self, BindFailure::Cloud(CloudError::Expired))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindStatus {
    Bound,
    AlreadyBound,
}

#[derive(Debug, Clone)]
pub struct BindReport {
    pub status: BindStatus,
    pub serial: String,
    pub vendor_code: i32,
    pub vendor_message: String,
}

impl BindReport {
    pub fn message(&self) -> String {
        match self.status {
            BindStatus::Bound => format!("{} is now on this account", self.serial),
            BindStatus::AlreadyBound => format!(
                "the cloud reports {} is already on the account; the camera password was not checked",
                self.serial
            ),
        }
    }
}

pub struct BindClient {
    http: reqwest::Client,
    base: String,
    identity: Identity,
    counter: AtomicU32,
}

impl BindClient {
    pub fn new(identity: Identity, base: impl Into<String>) -> Self {
        let base = base.into().trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .user_agent("proview-cctv-web/bind")
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("bind http client");
        Self {
            http,
            base: if base.is_empty() {
                super::envelope::ENVELOPE_BASE.to_string()
            } else {
                base
            },
            identity,
            counter: AtomicU32::new(0),
        }
    }

    pub fn from_client(client: &EnvelopeClient) -> Self {
        Self::new(client.identity().clone(), client.base().to_string())
    }

    pub async fn add_device(
        &self,
        access: &str,
        request: &AddDevice,
    ) -> Result<BindReport, BindFailure> {
        if !self.identity.is_complete() {
            return Err(BindFailure::Cloud(CloudError::Malformed(
                super::envelope::MISSING_CLOUD_IDENTITY.to_string(),
            )));
        }
        let counter = self.counter.fetch_add(1, Ordering::Relaxed) % 9_999_999 + 1;
        let signed = signed_request(&self.identity, counter, super::envelope::now_millis());
        let body = seal(&signed.key, &request.form()).map_err(BindFailure::Cloud)?;
        let url = format!(
            "{}/mdadd/{SIGNED_VERSION}/{}/{}.rs",
            self.base, signed.tsid, signed.sig
        );
        let response = self
            .http
            .post(url)
            .header("Accept", "*/*")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("uuid", &self.identity.uuid)
            .header("appKey", &self.identity.app_key)
            .header("Authorization", access)
            .body(body)
            .send()
            .await
            .map_err(|error| BindFailure::Cloud(CloudError::Unreachable(error.to_string())))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| BindFailure::Cloud(CloudError::Unreachable(error.to_string())))?;
        if !status.is_success() {
            return Err(BindFailure::Cloud(CloudError::Unreachable(format!(
                "mdadd HTTP {status}"
            ))));
        }
        let plain = if !text.trim().is_empty()
            && text.trim().chars().all(|value| value.is_ascii_hexdigit())
        {
            open(&signed.key, &text).map_err(BindFailure::Cloud)?
        } else {
            text
        };
        let envelope: Envelope = serde_json::from_str(&plain).map_err(|error| {
            BindFailure::Cloud(CloudError::Malformed(format!("mdadd: {error}")))
        })?;
        match envelope.code {
            ADD_DEVICE_OK | CODE_OK_LEGACY => Ok(BindReport {
                status: BindStatus::Bound,
                serial: request.serial.clone(),
                vendor_code: envelope.code,
                vendor_message: envelope.msg.unwrap_or_default(),
            }),
            ADD_DEVICE_ALREADY => Ok(BindReport {
                status: BindStatus::AlreadyBound,
                serial: request.serial.clone(),
                vendor_code: envelope.code,
                vendor_message: envelope.msg.unwrap_or_default(),
            }),
            ADD_DEVICE_USER_FAILED => Err(BindFailure::Vendor {
                code: envelope.code,
                message: envelope.msg.unwrap_or_default(),
            }),
            other => Err(BindFailure::Vendor {
                code: other,
                message: envelope.msg.unwrap_or_default(),
            }),
        }
    }

    pub async fn remove_device(&self, access: &str, serial: &str) -> Result<i32, BindFailure> {
        if !self.identity.is_complete() {
            return Err(BindFailure::Cloud(CloudError::Malformed(
                super::envelope::MISSING_CLOUD_IDENTITY.to_string(),
            )));
        }
        let counter = self.counter.fetch_add(1, Ordering::Relaxed) % 9_999_999 + 1;
        let signed = signed_request(&self.identity, counter, super::envelope::now_millis());
        let body = seal(&signed.key, &format!("uuid={}", url_encode(serial)))
            .map_err(BindFailure::Cloud)?;
        let url = format!(
            "{}/mdrm/{SIGNED_VERSION}/{}/{}.rs",
            self.base, signed.tsid, signed.sig
        );
        let response = self
            .http
            .post(url)
            .header("Accept", "*/*")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("uuid", &self.identity.uuid)
            .header("appKey", &self.identity.app_key)
            .header("Authorization", access)
            .body(body)
            .send()
            .await
            .map_err(|error| BindFailure::Cloud(CloudError::Unreachable(error.to_string())))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| BindFailure::Cloud(CloudError::Unreachable(error.to_string())))?;
        if !status.is_success() {
            return Err(BindFailure::Cloud(CloudError::Unreachable(format!(
                "mdrm HTTP {status}"
            ))));
        }
        let plain = if !text.trim().is_empty()
            && text.trim().chars().all(|value| value.is_ascii_hexdigit())
        {
            open(&signed.key, &text).map_err(BindFailure::Cloud)?
        } else {
            text
        };
        let envelope: Envelope = serde_json::from_str(&plain)
            .map_err(|error| BindFailure::Cloud(CloudError::Malformed(format!("mdrm: {error}"))))?;
        if envelope.code == CODE_OK || envelope.code == CODE_OK_LEGACY {
            return Ok(envelope.code);
        }
        Err(BindFailure::Vendor {
            code: envelope.code,
            message: envelope.msg.unwrap_or_default(),
        })
    }

    pub async fn vendor_sign_out(&self, access: &str) -> SignOutReply {
        let url = format!("{}/webservice/app/user/logOut", self.base);
        let attempt = self
            .http
            .post(&url)
            .header("Accept", "*/*")
            .header("Authorization", access)
            .header("Cookie", format!("accessToken={access}"))
            .header("Connection", "close")
            .body("")
            .send()
            .await;
        let response = match attempt {
            Ok(response) => response,
            Err(error) => {
                return SignOutReply {
                    http_status: 0,
                    vendor_code: None,
                    vendor_message: error.to_string(),
                }
            }
        };
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        let decoded = percent_decode(&text);
        let trimmed = decoded.trim();
        let envelope: Option<Envelope> = serde_json::from_str(trimmed).ok();
        SignOutReply {
            http_status: status,
            vendor_code: envelope.as_ref().map(|value| value.code),
            vendor_message: envelope
                .and_then(|value| value.msg)
                .unwrap_or_else(|| trimmed.chars().take(120).collect()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SignOutReply {
    pub http_status: u16,
    pub vendor_code: Option<i32>,
    pub vendor_message: String,
}

impl SignOutReply {
    pub fn is_ok(&self) -> bool {
        self.vendor_code == Some(CODE_OK) || self.vendor_code == Some(CODE_OK_LEGACY)
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or_default();
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn key_shift(index: usize) -> u32 {
    let key = QR_CODEC_KEY.as_bytes();
    key[index % key.len()] as u32
}

fn shift_char(value: u8, index: usize, forward: bool) -> Option<u8> {
    let shift = key_shift(index);
    let (base, modulus) = match value {
        b'0'..=b'9' => (b'0', 10u32),
        b'a'..=b'z' => (b'a', 26),
        b'A'..=b'Z' => (b'A', 26),
        _ => return None,
    };
    let amount = if modulus == 10 {
        shift % 10
    } else {
        shift % 26
    };
    let offset = (value - base) as u32;
    let moved = if forward {
        (offset + amount) % modulus
    } else {
        (offset + modulus - amount) % modulus
    };
    Some(base + moved as u8)
}

fn to62(value: u32) -> String {
    let mut out = String::new();
    let mut rest = value;
    loop {
        if rest < 26 {
            out.push((b'a' + rest as u8) as char);
            return out;
        }
        if rest < 52 {
            out.push((b'A' + (rest - 26) as u8) as char);
            return out;
        }
        if rest < 61 {
            out.push((b'1' + (rest - 52) as u8) as char);
            return out;
        }
        out.insert(0, '0');
        rest -= 61;
    }
}

fn from62(byte: u8) -> Option<u32> {
    match byte {
        b'a'..=b'z' => Some((byte - b'a') as u32),
        b'A'..=b'Z' => Some((byte - b'A') as u32 + 26),
        b'1'..=b'9' => Some((byte - b'1') as u32 + 52),
        _ => None,
    }
}

pub fn encode_qr_blob(plain: &str, offset: u32) -> String {
    let mut digits = String::with_capacity(plain.len() * 3 + 3);
    for byte in plain.bytes() {
        digits.push_str(&format!("{:03}", byte as u32 + offset));
    }
    let digitsum = format!("{offset:03}")
        .bytes()
        .map(|value| (value - b'0') as u32)
        .sum::<u32>()
        % 10;
    let mut shifted = String::with_capacity(digits.len() + 3);
    for value in digits.bytes() {
        shifted.push((b'0' + (((value - b'0') as u32 + digitsum) % 10) as u8) as char);
    }
    shifted.push_str(&format!("{offset:03}"));
    let bytes = shifted.as_bytes();
    let mut packed = String::with_capacity(shifted.len());
    let mut index = 0;
    while index < bytes.len() {
        let value = if bytes.len() - index >= 2 {
            let pair = (bytes[index] - b'0') as u32 * 10 + (bytes[index + 1] - b'0') as u32;
            index += 2;
            pair
        } else {
            index += 1;
            (bytes[index - 1] - b'0') as u32
        };
        packed.push_str(&to62(value));
    }
    let mut out: Vec<u8> = packed
        .bytes()
        .enumerate()
        .map(|(index, value)| shift_char(value, index, true).unwrap_or(value))
        .collect();
    out.push(QR_CODEC_MARK as u8);
    String::from_utf8(out).unwrap_or_default()
}

fn unpack_digits(packed: &str, last_single: bool) -> Option<String> {
    let bytes = packed.as_bytes();
    let mut out = String::with_capacity(bytes.len() * 2);
    let mut index = 0;
    while index < bytes.len() {
        if last_single && index + 1 == bytes.len() {
            let value = from62(bytes[index])?;
            if value > 9 {
                return None;
            }
            out.push((b'0' + value as u8) as char);
            index += 1;
            continue;
        }
        if bytes[index] == b'0' {
            let next = *bytes.get(index + 1)?;
            out.push_str(&format!("{:02}", 61 + from62(next)?));
            index += 2;
            continue;
        }
        out.push_str(&format!("{:02}", from62(bytes[index])?));
        index += 1;
    }
    Some(out)
}

pub fn decode_qr_blob(encoded: &str) -> Option<String> {
    let trimmed = encoded.trim();
    let body = trimmed.strip_suffix(QR_CODEC_MARK)?;
    let unshifted: Vec<u8> = body
        .bytes()
        .enumerate()
        .map(|(index, value)| shift_char(value, index, false))
        .collect::<Option<Vec<u8>>>()?;
    let packed = std::str::from_utf8(&unshifted).ok()?;
    for last_single in [false, true] {
        let digits = match unpack_digits(packed, last_single) {
            Some(value) => value,
            None => continue,
        };
        if digits.len() < 6 || (digits.len() - 3) % 3 != 0 {
            continue;
        }
        let tail = &digits[digits.len() - 3..];
        let offset: u32 = match tail.parse() {
            Ok(value) => value,
            Err(_) => continue,
        };
        if offset >= QR_OFFSET_LIMIT {
            continue;
        }
        let digitsum = tail.bytes().map(|value| (value - b'0') as u32).sum::<u32>() % 10;
        let mut plain_digits = String::with_capacity(digits.len() - 3);
        for value in digits[..digits.len() - 3].bytes() {
            let digit = (value - b'0') as u32;
            plain_digits.push((b'0' + ((digit + 10 - digitsum) % 10) as u8) as char);
        }
        let mut bytes = Vec::with_capacity(plain_digits.len() / 3);
        let mut broken = false;
        for chunk in plain_digits.as_bytes().chunks(3) {
            let text = match std::str::from_utf8(chunk) {
                Ok(value) => value,
                Err(_) => {
                    broken = true;
                    break;
                }
            };
            let raw: u32 = match text.parse() {
                Ok(value) => value,
                Err(_) => {
                    broken = true;
                    break;
                }
            };
            if raw < offset || raw - offset > 255 {
                broken = true;
                break;
            }
            bytes.push((raw - offset) as u8);
        }
        if broken {
            continue;
        }
        return Some(String::from_utf8_lossy(&bytes).to_string());
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrDevice {
    pub serial: String,
    pub username: String,
    pub password: String,
    pub kind: Option<u32>,
    pub share_time: Option<i64>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QrError {
    Empty,
    NotQr,
    BadSerial(String),
}

impl std::fmt::Display for QrError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QrError::Empty => write!(formatter, "the QR payload is empty"),
            QrError::NotQr => write!(
                formatter,
                "this QR payload is not a PROVIEW device code; the vendor blob did not decode"
            ),
            QrError::BadSerial(found) => {
                write!(formatter, "{found:?} is not a device serial")
            }
        }
    }
}

fn unwrap_share_link(payload: &str) -> String {
    if let Some(index) = payload.find("shareInfo=") {
        let value = &payload[index + "shareInfo=".len()..];
        let value = value.split('&').next().unwrap_or(value);
        return percent_decode(value);
    }
    payload.to_string()
}

pub fn serial_ok(serial: &str) -> bool {
    let length = serial.len();
    (8..=32).contains(&length) && serial.bytes().all(|value| value.is_ascii_alphanumeric())
}

#[derive(Deserialize)]
struct ShareFields {
    #[serde(default, alias = "sn", alias = "serial")]
    dev_id: Option<String>,
    #[serde(default, rename = "devId")]
    dev_id_camel: Option<String>,
    #[serde(default, rename = "loginName")]
    login_name: Option<String>,
    #[serde(default)]
    pwd: Option<String>,
    #[serde(default, rename = "devType")]
    dev_type: Option<u32>,
    #[serde(default, rename = "shareTimes")]
    share_times: Option<i64>,
    #[serde(default)]
    name: Option<String>,
}

pub fn parse_qr(payload: &str) -> Result<QrDevice, QrError> {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return Err(QrError::Empty);
    }
    let candidate = unwrap_share_link(trimmed);
    let text = match decode_qr_blob(&candidate) {
        Some(value) => value,
        None if candidate.contains(',') || candidate.trim_start().starts_with('{') => candidate,
        None => return Err(QrError::NotQr),
    };
    let text = text.trim();
    if text.is_empty() {
        return Err(QrError::NotQr);
    }
    if text.starts_with('{') {
        let fields: ShareFields = serde_json::from_str(text).map_err(|_| QrError::NotQr)?;
        let serial = fields.dev_id.or(fields.dev_id_camel).unwrap_or_default();
        if !serial_ok(&serial) {
            return Err(QrError::BadSerial(serial));
        }
        return Ok(QrDevice {
            serial,
            username: fields.login_name.unwrap_or_default(),
            password: fields.pwd.unwrap_or_default(),
            kind: fields.dev_type,
            share_time: fields.share_times,
            name: fields.name,
        });
    }
    let mut parts = text.split(',');
    let serial = parts.next().unwrap_or_default().trim().to_string();
    if !serial_ok(&serial) {
        return Err(QrError::BadSerial(serial));
    }
    let username = parts.next().unwrap_or_default().trim().to_string();
    let password = parts.next().unwrap_or_default().trim().to_string();
    let kind = parts.next().and_then(|value| value.trim().parse().ok());
    let share_time = parts.next().and_then(|value| value.trim().parse().ok());
    Ok(QrDevice {
        serial,
        username,
        password,
        kind,
        share_time,
        name: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VENDOR_BLOBS: [(&str, &str, u32); 6] = [
        ("z1evz", "a", 0),
        ("z1e3Rh1Ugz", "abc", 0),
        ("wVvz", "A", 0),
        ("u1ovz", "0", 0),
        ("a1Zvbiiz", "abc", 1),
        ("lkrBKaz", "abc", 2),
    ];

    #[test]
    fn decodes_the_vendor_blobs_produced_by_fun_encdevinfo() {
        for (blob, plain, _) in VENDOR_BLOBS {
            assert_eq!(decode_qr_blob(blob).as_deref(), Some(plain), "blob {blob}");
        }
        assert_eq!(
            decode_qr_blob("vWxa0hFw0e5a1kxfwidLPl8FbV9cgzrM1Dgvz").as_deref(),
            Some("7f92970ddf5282743hrc")
        );
        assert_eq!(
            decode_qr_blob(
                "8DIC0Ga40A1eX8C9dG2M9oV1B1yX9A0gH9L9pX1UAh3KBJ9KYI0CDw9A0KI9L9pv1VqV3Lbtz"
            )
            .as_deref(),
            Some("AASN000000000001,,,2,1793109504")
        );
    }

    #[test]
    fn encoding_matches_the_vendor_function_byte_for_byte() {
        for (blob, plain, offset) in VENDOR_BLOBS {
            assert_eq!(
                encode_qr_blob(plain, offset),
                blob,
                "plain {plain} offset {offset}"
            );
        }
        assert_eq!(
            encode_qr_blob("7f92970ddf5282743hrc", 0),
            "vWxa0hFw0e5a1kxfwidLPl8FbV9cgzrM1Dgvz"
        );
    }

    #[test]
    fn round_trips_arbitrary_text_at_every_offset() {
        let text = "sn12345,admin,pa55,0,1751799510";
        for offset in [0u32, 7, 300, 743] {
            let blob = encode_qr_blob(text, offset);
            assert_eq!(
                decode_qr_blob(&blob).as_deref(),
                Some(text),
                "offset {offset}"
            );
        }
    }

    #[test]
    fn rejects_a_blob_without_the_vendor_terminator() {
        assert_eq!(decode_qr_blob("z1e3Rh1Ug"), None);
        assert_eq!(decode_qr_blob(""), None);
        assert_eq!(decode_qr_blob("!!!"), None);
    }

    #[test]
    fn parses_the_plain_comma_form() {
        let device = parse_qr("7f92970ddf5282743hrc,admin,,0,1793109504").unwrap();
        assert_eq!(device.serial, "7f92970ddf5282743hrc");
        assert_eq!(device.username, "admin");
        assert_eq!(device.password, "");
        assert_eq!(device.kind, Some(0));
        assert_eq!(device.share_time, Some(1793109504));
    }

    #[test]
    fn parses_an_encoded_vendor_blob_into_the_serial() {
        let blob = encode_qr_blob("7f92970ddf5282743hrc,,hunter2,0,1793109504", 334);
        let device = parse_qr(&blob).unwrap();
        assert_eq!(device.serial, "7f92970ddf5282743hrc");
        assert_eq!(device.password, "hunter2");
    }

    #[test]
    fn parses_the_share_link_wrapped_json() {
        let json = r#"{"devType":0,"userId":"u1","loginName":"admin","pwd":"pw","devId":"AASN000000000001","shareTimes":1751799510}"#;
        let blob = encode_qr_blob(json, 12);
        let device = parse_qr(&format!("https://d.xmeye.net/CSee?shareInfo={blob}")).unwrap();
        assert_eq!(device.serial, "AASN000000000001");
        assert_eq!(device.username, "admin");
        assert_eq!(device.password, "pw");
        assert_eq!(device.share_time, Some(1751799510));
    }

    #[test]
    fn rejects_payloads_that_carry_no_serial() {
        assert_eq!(parse_qr("   "), Err(QrError::Empty));
        assert!(matches!(
            parse_qr("hello,world"),
            Err(QrError::BadSerial(_))
        ));
        assert!(matches!(
            parse_qr("{\"pwd\":\"x\"}"),
            Err(QrError::BadSerial(_))
        ));
    }

    #[test]
    fn a_payload_with_characters_outside_the_codec_alphabet_fails_instead_of_panicking() {
        assert_eq!(decode_qr_blob("a b/z"), None);
        assert_eq!(decode_qr_blob("see https://x.test/a b-z"), None);
        assert!(matches!(
            parse_qr("see https://x.test/a b-z"),
            Err(QrError::NotQr)
        ));
        assert!(matches!(
            parse_qr("sn,admin,passz"),
            Err(QrError::BadSerial(_))
        ));
    }

    #[test]
    fn a_blob_shaped_payload_that_does_not_decode_is_never_read_as_a_serial() {
        let payload = "AASN000000000777z";
        assert_eq!(decode_qr_blob(payload), None);
        assert_eq!(parse_qr(payload), Err(QrError::NotQr));
    }

    #[test]
    fn serial_validation_covers_the_bounds_the_routes_rely_on() {
        assert!(serial_ok("AASN000000000001"));
        assert!(serial_ok("7f92970ddf5282743hrc"));
        assert!(!serial_ok("no"));
        assert!(!serial_ok(""));
        assert!(!serial_ok("AASN 0000 0001"));
        assert!(!serial_ok(&"a".repeat(33)));
    }

    #[test]
    fn add_device_form_matches_the_vendor_field_order() {
        let request = AddDevice {
            serial: "AASN000000000001".to_string(),
            username: "admin".to_string(),
            password: "p@ss word".to_string(),
            account_id: "c6f8c33a".to_string(),
            nickname: "front gate".to_string(),
            kind: 0,
            ip: "0.0.0.0".to_string(),
            port: 34567,
        };
        assert_eq!(
            request.form(),
            "uuid=AASN000000000001&username=admin&password=p%40ss%20word&userId=c6f8c33a&nickname=front%20gate&type=0&ip=0.0.0.0&port=34567"
        );
    }
}
