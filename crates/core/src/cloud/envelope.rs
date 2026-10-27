use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::{Aes128, Block};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use md5::{Digest, Md5};
use rand::rngs::OsRng;
use rand::RngCore;
use rsa::pkcs1v15::Pkcs1v15Encrypt;
use rsa::pkcs8::DecodePublicKey;
use rsa::RsaPublicKey;
use serde::Deserialize;

use super::client::{CloudError, CloudResult};
use super::types::{user_message, CloudDevice, Envelope, CODE_BAD_CREDENTIALS, CODE_TOKEN_EXPIRED};

pub const ENVELOPE_BASE: &str = "https://rs.xmeye.net";
pub const PLATFORM_BASE: &str = "https://boss22-api.xmcsrv.net";
pub const PLATFORM_ORIGIN: &str = "https://boss22.xmcsrv.net";
pub const PLATFORM_SCHEME: &str = "icsee.boss.jftech.com";
pub const PLATFORM_VERSION: &str = "1.0.2";
pub const SIGNED_VERSION: &str = "v3";
pub const COUNTER_WRAP: u32 = 38528;
pub const CODE_BAD_CREDENTIALS_VENDOR: i32 = -83000;
pub const SCRAMBLE: u32 = 4;

const ACCEPT: &str = "text/html, image/gif, image/jpeg, *; q=.2, */*; q=.2";
const USER_AGENT: &str = "proview-cctv-web/0.2";
const BLOCK: usize = 16;

#[derive(Clone, Default)]
pub struct Identity {
    pub uuid: String,
    pub app_key: String,
    pub app_secret: String,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Identity")
            .field("uuid", &self.uuid)
            .field("app_key", &self.app_key)
            .field("app_secret", &redacted(&self.app_secret))
            .finish()
    }
}

fn redacted(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    format!("{}..{}", &value[..4.min(value.len())], value.len())
}

impl Identity {
    pub fn new(
        uuid: impl Into<String>,
        app_key: impl Into<String>,
        app_secret: impl Into<String>,
    ) -> Self {
        Self {
            uuid: uuid.into(),
            app_key: app_key.into(),
            app_secret: app_secret.into(),
        }
    }

    pub fn from_env() -> Self {
        Self {
            uuid: env("PROVIEW_APP_UUID"),
            app_key: env("PROVIEW_APP_KEY"),
            app_secret: env("PROVIEW_APP_SECRET"),
        }
    }

    pub fn is_complete(&self) -> bool {
        !self.uuid.trim().is_empty()
            && !self.app_key.trim().is_empty()
            && !self.app_secret.trim().is_empty()
    }
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub tsid: String,
    pub sig: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Claims {
    pub exp: Option<i64>,
    pub iat: Option<i64>,
    pub account: Option<String>,
    #[serde(default, rename = "appKey")]
    pub app_key: Option<String>,
}

pub fn url_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(byte, b'/' | b'.' | b'!' | b'-' | b'_' | b'*' | b'(' | b')');
        if keep {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

pub fn scramble(input: &str, k: u32) -> String {
    let mut bytes = input.as_bytes().to_vec();
    let len = bytes.len();
    for index in 0..len {
        let mirror = len - 1 - index;
        let swap = k != 0 && (index as u32 % k) > ((len - index) as u32 % k);
        if swap {
            bytes.swap(index, mirror);
        } else {
            bytes[index] = bytes[mirror];
        }
    }
    String::from_utf8(bytes).unwrap_or_default()
}

pub fn signature(source: &str, k: u32) -> String {
    let mut text = String::with_capacity(source.len() * 2);
    text.push_str(source);
    text.extend(scramble(source, k).chars().rev());
    let mut hasher = Md5::new();
    hasher.update(text.as_bytes());
    hex_lower(hasher.finalize().as_slice())
}

pub fn body_key(tsid: &str, app_secret: &str) -> String {
    let half = tsid.len() / 2;
    let mut key = String::with_capacity(BLOCK);
    key.push_str(tsid.get(half..).unwrap_or_default());
    key.push_str(app_secret);
    let mut counter = 0;
    while key.len() < BLOCK {
        key.push_str(&counter.to_string());
        counter += 1;
    }
    key.truncate(BLOCK);
    key
}

pub fn signed_request(identity: &Identity, counter: u32, millis: u64) -> Signed {
    let tsid = format!("{counter:07}{millis}");
    let source = format!(
        "{}{}{}{}",
        identity.uuid, identity.app_key, identity.app_secret, tsid
    );
    Signed {
        key: body_key(&tsid, &identity.app_secret),
        sig: signature(&source, SCRAMBLE),
        tsid,
    }
}

fn cipher_of(key: &str) -> CloudResult<Aes128> {
    let bytes = key.as_bytes();
    if bytes.len() != BLOCK {
        return Err(CloudError::Malformed(format!(
            "body key must be 16 bytes, got {}",
            bytes.len()
        )));
    }
    Ok(Aes128::new_from_slice(bytes).expect("aes-128 accepts 16 bytes"))
}

pub fn seal(key: &str, plain: &str) -> CloudResult<String> {
    let cipher = cipher_of(key)?;
    let mut buffer = plain.as_bytes().to_vec();
    let pad = BLOCK - buffer.len() % BLOCK;
    buffer.extend(std::iter::repeat_n(pad as u8, pad));
    for chunk in buffer.chunks_mut(BLOCK) {
        cipher.encrypt_block(Block::from_mut_slice(chunk));
    }
    Ok(hex_upper(&buffer))
}

pub fn open(key: &str, body: &str) -> CloudResult<String> {
    if !is_hex_block(body) {
        return Err(CloudError::Malformed(format!(
            "reply is not an even hex block: {}",
            body.trim().chars().take(40).collect::<String>()
        )));
    }
    let trimmed = body.trim();
    let mut buffer = Vec::with_capacity(trimmed.len() / 2);
    let bytes = trimmed.as_bytes();
    for pair in bytes.chunks(2) {
        let text = std::str::from_utf8(pair)
            .map_err(|_| CloudError::Malformed("reply is not hex".to_string()))?;
        buffer.push(
            u8::from_str_radix(text, 16)
                .map_err(|_| CloudError::Malformed("reply is not hex".to_string()))?,
        );
    }
    let cipher = cipher_of(key)?;
    for chunk in buffer.chunks_mut(BLOCK) {
        cipher.decrypt_block(Block::from_mut_slice(chunk));
    }
    let pad = *buffer
        .last()
        .ok_or_else(|| CloudError::Malformed("empty reply".to_string()))? as usize;
    if pad == 0 || pad > BLOCK || pad > buffer.len() {
        return Err(CloudError::Malformed(
            "reply padding is invalid".to_string(),
        ));
    }
    let split = buffer.len() - pad;
    if buffer[split..].iter().any(|byte| *byte as usize != pad) {
        return Err(CloudError::Malformed(
            "reply padding is invalid".to_string(),
        ));
    }
    buffer.truncate(split);
    String::from_utf8(buffer).map_err(|_| CloudError::Malformed("reply is not utf-8".to_string()))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn hex_upper(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02X}"));
    }
    out
}

fn is_hex_block(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && trimmed.len() % (BLOCK * 2) == 0
        && trimmed.chars().all(|value| value.is_ascii_hexdigit())
}

pub fn login_error(code: i32, vendor: Option<&str>) -> CloudError {
    match code {
        CODE_BAD_CREDENTIALS | CODE_BAD_CREDENTIALS_VENDOR => CloudError::Credentials,
        other => CloudError::Rejected(user_message(other, vendor)),
    }
}

pub fn claims(token: &str) -> Option<Claims> {
    let payload = token.split('.').nth(1)?;
    let raw = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&raw).ok()
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

pub struct EnvelopeClient {
    http: reqwest::Client,
    base: String,
    platform: String,
    identity: Identity,
    counter: AtomicU32,
}

impl EnvelopeClient {
    pub fn new(identity: Identity) -> Self {
        Self::with_base(identity, ENVELOPE_BASE)
    }

    pub fn with_base(identity: Identity, base: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("envelope http client");
        let base = base.into().trim_end_matches('/').to_string();
        Self {
            http,
            base: if base.is_empty() {
                ENVELOPE_BASE.to_string()
            } else {
                base
            },
            platform: PLATFORM_BASE.to_string(),
            identity,
            counter: AtomicU32::new(0),
        }
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn next(&self) -> Signed {
        let counter = self
            .counter
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                let next = current + 1;
                Some(if next >= COUNTER_WRAP { 1 } else { next })
            })
            .map(|previous| {
                if previous + 1 >= COUNTER_WRAP {
                    1
                } else {
                    previous + 1
                }
            })
            .unwrap_or(1);
        signed_request(&self.identity, counter, now_millis())
    }

    fn signed(
        &self,
        endpoint: &str,
        body: Option<&str>,
    ) -> CloudResult<(reqwest::RequestBuilder, String)> {
        let signed = self.next();
        let url = format!(
            "{}/{endpoint}/{}/{}/{}.rs",
            self.base, SIGNED_VERSION, signed.tsid, signed.sig
        );
        let mut request = self
            .http
            .post(url)
            .header("Accept", ACCEPT)
            .header("Accept-Charset", "utf-8")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("uuid", &self.identity.uuid)
            .header("appKey", &self.identity.app_key);
        if let Some(plain) = body {
            request = request.body(seal(&signed.key, plain)?);
        }
        Ok((request, signed.key))
    }

    async fn finish(response: reqwest::Response, key: &str) -> CloudResult<String> {
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        if !status.is_success() {
            return Err(CloudError::Unreachable(format!("HTTP {status}")));
        }
        if is_hex_block(&text) {
            return open(key, &text);
        }
        Ok(text)
    }

    pub async fn login_envelope(&self, account: &str, password: &str) -> CloudResult<Tokens> {
        if !self.identity.is_complete() {
            return Err(CloudError::Malformed(
                "the cloud login needs the app uuid and app key; set PROVIEW_APP_UUID and PROVIEW_APP_KEY"
                    .to_string(),
            ));
        }
        let body = format!("account={}&pass={}", account.trim(), url_encode(password));
        let (request, key) = self.signed("loginExist", Some(&body))?;
        let response = request
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let plain = Self::finish(response, &key).await?;
        let envelope: Envelope = serde_json::from_str(&plain)
            .map_err(|error| CloudError::Malformed(format!("login: {error}")))?;
        if !envelope.is_ok() {
            return Err(login_error(envelope.code, envelope.msg.as_deref()));
        }
        let access = envelope
            .data
            .get("accessToken")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| CloudError::Malformed("login reply is missing accessToken".to_string()))?
            .to_string();
        let refresh = envelope
            .data
            .get("refreshToken")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let expires_at = claims(&access).and_then(|claims| claims.exp);
        Ok(Tokens {
            access,
            refresh,
            expires_at,
        })
    }

    pub async fn device_token(&self, access: &str, device_id: &str) -> CloudResult<String> {
        let exchange = self
            .http
            .post(format!("{}/account/userToken/login", self.platform))
            .header("Content-Type", "application/json")
            .header("Origin", PLATFORM_ORIGIN)
            .header("Accept", "*/*")
            .header("User-Agent", "Mozilla/5.0")
            .json(&serde_json::json!({
                "request": {
                    "authorization": access,
                    "appKey": self.identity.app_key,
                    "appScheme": PLATFORM_SCHEME,
                    "appVer": PLATFORM_VERSION,
                }
            }))
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let status = exchange.status();
        let text = exchange
            .text()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        if !status.is_success() {
            return Err(CloudError::Unreachable(format!(
                "platform exchange HTTP {status}"
            )));
        }
        let envelope: Envelope = serde_json::from_str(&text)
            .map_err(|error| CloudError::Malformed(format!("platform exchange: {error}")))?;
        if !envelope.is_ok() {
            return Err(CloudError::Rejected(user_message(
                envelope.code,
                envelope.msg.as_deref(),
            )));
        }
        let platform_access = envelope
            .data
            .get("accessToken")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                CloudError::Malformed("platform exchange is missing accessToken".to_string())
            })?;
        let public_key = envelope
            .data
            .get("publicKey")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                CloudError::Malformed("platform exchange is missing publicKey".to_string())
            })?;
        let params = serde_json::json!({ "deviceId": device_id });
        let reply = self
            .sealed("/deviceToken/get", &params, platform_access, public_key)
            .await?;
        device_token_of(&reply)
    }

    async fn sealed(
        &self,
        path: &str,
        params: &serde_json::Value,
        access: &str,
        public_key: &str,
    ) -> CloudResult<String> {
        let der = STANDARD
            .decode(public_key.trim())
            .map_err(|_| CloudError::Malformed("platform publicKey is not base64".to_string()))?;
        let key = RsaPublicKey::from_public_key_der(&der).map_err(|error| {
            CloudError::Malformed(format!("platform publicKey is not DER: {error}"))
        })?;
        let mut session = [0u8; BLOCK];
        OsRng.fill_bytes(&mut session);
        let sealed_key = key
            .encrypt(
                &mut OsRng,
                Pkcs1v15Encrypt,
                STANDARD.encode(session).as_bytes(),
            )
            .map_err(|error| {
                CloudError::Malformed(format!("platform key sealing failed: {error}"))
            })?;
        let plain =
            serde_json::to_vec(params).map_err(|error| CloudError::Malformed(error.to_string()))?;
        let mut buffer = plain;
        let pad = BLOCK - buffer.len() % BLOCK;
        buffer.extend(std::iter::repeat_n(pad as u8, pad));
        let cipher = Aes128::new_from_slice(&session).expect("aes-128 accepts 16 bytes");
        for chunk in buffer.chunks_mut(BLOCK) {
            cipher.encrypt_block(Block::from_mut_slice(chunk));
        }
        let response = self
            .http
            .post(format!("{}{}", self.platform, path))
            .header("Content-Type", "application/json")
            .header("Origin", PLATFORM_ORIGIN)
            .header("Accept", "*/*")
            .header("User-Agent", "Mozilla/5.0")
            .header("Authorization", access)
            .header("Key", STANDARD.encode(sealed_key))
            .json(&serde_json::json!({ "req": STANDARD.encode(buffer) }))
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        if !status.is_success() {
            return Err(CloudError::Unreachable(format!(
                "sealed call HTTP {status}"
            )));
        }
        Ok(text)
    }

    pub async fn device_list(&self, access: &str) -> CloudResult<Vec<CloudDevice>> {
        let (request, key) = self.signed("msdlistCode", None)?;
        let response = request
            .header("Authorization", access)
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let plain = Self::finish(response, &key).await?;
        let envelope: Envelope = serde_json::from_str(&plain)
            .map_err(|error| CloudError::Malformed(format!("msdlistCode: {error}")))?;
        if !envelope.is_ok() {
            return Err(CloudError::Rejected(user_message(
                envelope.code,
                envelope.msg.as_deref(),
            )));
        }
        let list = envelope
            .data
            .get("mine")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
        serde_json::from_value(list)
            .map_err(|error| CloudError::Malformed(format!("msdlistCode devices: {error}")))
    }

    pub async fn refresh(&self, refresh: &str) -> CloudResult<Tokens> {
        let body = format!("refreshToken={}", url_encode(refresh));
        let (request, key) = self.signed("refreshToken", Some(&body))?;
        let response = request
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let plain = Self::finish(response, &key).await?;
        let envelope: Envelope = serde_json::from_str(&plain)
            .map_err(|error| CloudError::Malformed(format!("refreshToken: {error}")))?;
        if !envelope.is_ok() {
            return Err(if envelope.code == CODE_TOKEN_EXPIRED {
                CloudError::Expired
            } else {
                CloudError::Rejected(user_message(envelope.code, envelope.msg.as_deref()))
            });
        }
        let access = envelope
            .data
            .get("accessToken")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let next = envelope
            .data
            .get("refreshToken")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(refresh)
            .to_string();
        Ok(Tokens {
            expires_at: claims(&access).and_then(|claims| claims.exp),
            access,
            refresh: next,
        })
    }
}

pub fn device_token_of(reply: &str) -> CloudResult<String> {
    let value: serde_json::Value = serde_json::from_str(reply)
        .map_err(|error| CloudError::Malformed(format!("device token: {error}")))?;
    if let Some(token) = value
        .get("data")
        .and_then(|data| data.get("token"))
        .or_else(|| value.get("token"))
        .and_then(serde_json::Value::as_str)
    {
        return Ok(token.to_string());
    }
    if let Some(code) = value.get("code").and_then(serde_json::Value::as_i64) {
        let msg = value.get("msg").and_then(serde_json::Value::as_str);
        return Err(CloudError::Rejected(user_message(code as i32, msg)));
    }
    let keys: Vec<String> = value
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default();
    Err(CloudError::Malformed(format!(
        "device token reply has no token; keys {keys:?}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scramble_mirrors_the_vendor_way() {
        assert_eq!(scramble("", SCRAMBLE), "");
        assert_eq!(scramble("ab", SCRAMBLE), "bb");
        assert_eq!(scramble("abcd", SCRAMBLE), "dccd");
        assert_eq!(scramble("0123456789abcdef", SCRAMBLE), "fedcba9889abcdef");
    }

    #[test]
    fn body_key_pads_short_material() {
        assert_eq!(body_key("00000021793070132241", ""), "3070132241012345");
        assert_eq!(
            body_key("00000031793070268874", "fedcba98765432100123456789abcdef"),
            "3070268874fedcba"
        );
    }

    #[test]
    fn seal_round_trips_and_stays_uppercase() {
        let sealed = seal("3070132241012345", "account=a@b.c&pass=x").unwrap();
        assert!(sealed
            .chars()
            .all(|value| !value.is_ascii_alphabetic() || value.is_ascii_uppercase()));
        assert_eq!(
            open("3070132241012345", &sealed).unwrap(),
            "account=a@b.c&pass=x"
        );
        assert!(open("3070132241012345", "AB").is_err());
        assert!(seal("short", "x").is_err());
    }

    #[test]
    fn url_encode_keeps_the_vendor_set() {
        assert_eq!(url_encode("a@b.c"), "a%40b.c");
        assert_eq!(url_encode("Tu!long-1_2*3(4)"), "Tu!long-1_2*3(4)");
        assert_eq!(url_encode("a b&c=d"), "a%20b%26c%3Dd");
    }

    #[test]
    fn login_errors_separate_credentials_from_refusals() {
        assert!(matches!(login_error(4000, None), CloudError::Credentials));
        assert!(matches!(
            login_error(-83000, Some("username or password error")),
            CloudError::Credentials
        ));
        match login_error(5004, Some("SIGNATURE_ERROR")) {
            CloudError::Rejected(message) => assert!(message.contains("SIGNATURE_ERROR")),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn identity_needs_all_three_platform_strings() {
        assert!(!Identity::new("uuid", "key", "").is_complete());
        assert!(!Identity::new("", "key", "secret").is_complete());
        assert!(!Identity::new("uuid", "", "secret").is_complete());
        assert!(Identity::new("uuid", "key", "secret").is_complete());
    }

    #[test]
    fn debug_never_prints_the_secret() {
        let identity = Identity::new("uuid-value", "key-value", "secret-value");
        let text = format!("{identity:?}");
        assert!(!text.contains("secret-value"), "{text}");
        assert!(text.contains("uuid-value"));
    }

    #[test]
    fn device_token_reply_is_read_defensively() {
        assert_eq!(
            device_token_of(r#"{"code":2000,"data":{"token":"abc"}}"#).unwrap(),
            "abc"
        );
        assert_eq!(device_token_of(r#"{"token":"def"}"#).unwrap(), "def");
        let rejected = device_token_of(r#"{"code":-91002,"msg":"deviceId"}"#).unwrap_err();
        assert!(rejected.to_string().contains("deviceId"));
        let odd = device_token_of(r#"{"data":{}}"#).unwrap_err();
        assert!(odd.to_string().contains("keys"));
    }

    #[test]
    fn claims_decode_from_a_jwt() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"exp":123,"iat":1,"account":"ab","appKey":"k"}"#);
        let token = format!("h.{payload}.s");
        let parsed = claims(&token).expect("claims");
        assert_eq!(parsed.exp, Some(123));
        assert_eq!(parsed.account.as_deref(), Some("ab"));
    }
}
