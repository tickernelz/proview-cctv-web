use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE;
use base64::Engine;
use serde::Deserialize;
use serde_json::json;

use super::client::{CloudError, CloudResult};
use super::envelope::{signed_request, Identity, Signed};

pub const TOKEN_BASE: &str = "https://pub-token.xmeye.net";
pub const TOKEN_PATH: &str = "/reqToken";
pub const TOKEN_SERVICE: &str = "dss";
const TIMEOUT_SECONDS: u64 = 20;
const USER_AGENT: &str = "proview-cctv-web/0.3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DssToken {
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DssClaims {
    pub app: String,
    pub sn: String,
    pub expires: u64,
    pub service: String,
    pub media_ip: String,
    pub tag: String,
}

pub fn token_body(
    identity: &Identity,
    sn: &str,
    user: &str,
    password: &str,
    counter: u32,
    millis: u64,
) -> (String, Signed) {
    let signed = signed_request(identity, counter, millis);
    let body = json!({
        "sn": sn,
        "user": user,
        "pass": password,
        "service": TOKEN_SERVICE,
        "uuid": identity.uuid,
        "appkey": identity.app_key,
        "tm": signed.tsid,
        "sign": signed.sig,
    })
    .to_string();
    (body, signed)
}

#[derive(Deserialize)]
struct TokenReply {
    #[serde(default)]
    ret: i32,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    token: String,
}

pub fn read_reply(text: &str) -> CloudResult<DssToken> {
    let reply: TokenReply = serde_json::from_str(text)
        .map_err(|error| CloudError::Malformed(format!("{error}: {text}")))?;
    if reply.ret != 200 {
        return Err(CloudError::Rejected(format!(
            "token endpoint answered ret {}: {}",
            reply.ret, reply.msg
        )));
    }
    let raw = reply.token.trim();
    if raw.is_empty() {
        return Err(CloudError::Malformed("token endpoint answered without a token".into()));
    }
    Ok(DssToken { raw: raw.to_string() })
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let pair = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            out.push(u8::from_str_radix(pair, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

impl DssToken {
    pub fn claims(&self) -> Option<DssClaims> {
        let (head, _signature) = self.raw.split_once('.')?;
        let decoded = percent_decode(head)?;
        let payload = URL_SAFE.decode(decoded).ok()?;
        let text = String::from_utf8(payload).ok()?;
        let fields: Vec<&str> = text.split('|').collect();
        if fields.len() < 7 {
            return None;
        }
        Some(DssClaims {
            app: fields[0].to_string(),
            sn: fields[1].to_string(),
            expires: fields[3].parse().unwrap_or_default(),
            service: fields[4].to_string(),
            media_ip: fields[5].to_string(),
            tag: fields[6].to_string(),
        })
    }
}

pub struct DssClient {
    identity: Identity,
    base: String,
    http: reqwest::Client,
}

impl DssClient {
    pub fn new(identity: Identity) -> Self {
        Self::with_base(identity, TOKEN_BASE)
    }

    pub fn with_base(identity: Identity, base: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(TIMEOUT_SECONDS))
            .build()
            .unwrap_or_default();
        Self {
            identity,
            base: base.into().trim_end_matches('/').to_string(),
            http,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub async fn token(&self, sn: &str, user: &str, password: &str) -> CloudResult<DssToken> {
        if !self.identity.is_complete() {
            return Err(CloudError::Malformed(
                super::envelope::MISSING_CLOUD_IDENTITY.to_string(),
            ));
        }
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|span| span.as_millis() as u64)
            .unwrap_or_default();
        let (body, _signed) = token_body(&self.identity, sn, user, password, 1, millis);
        let response = self
            .http
            .post(format!("{}{}", self.base, TOKEN_PATH))
            .header("Content-Type", "application/json")
            .header("Accept", "*/*")
            .body(body)
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        if !status.is_success() {
            return Err(CloudError::Malformed(format!(
                "token endpoint answered {status}: {text}"
            )));
        }
        read_reply(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "YWNiZTQxOWI0NnxaSEE0MDA2UFJPQkUwMDAxfFptRnJaUT09fDE3OTMxNDQ5MDQxMDR8ZHNzfDEwMy44NS4xMjIuNjZ8bWQ1eA%3D%3D.64e14edd041fce1af2627fa69264575d";

    fn identity() -> Identity {
        Identity::new("uuid-0001", "appkey-0001", "secret-0001")
    }

    #[test]
    fn body_carries_the_dss_service_and_the_signed_tm() {
        let (body, signed) = token_body(&identity(), "SN0001", "admin", "pw", 1, 1_793_144_904_104);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["sn"], "SN0001");
        assert_eq!(parsed["user"], "admin");
        assert_eq!(parsed["pass"], "pw");
        assert_eq!(parsed["service"], "dss");
        assert_eq!(parsed["uuid"], "uuid-0001");
        assert_eq!(parsed["appkey"], "appkey-0001");
        assert_eq!(parsed["tm"], signed.tsid);
        assert_eq!(parsed["sign"], signed.sig);
        assert_eq!(parsed["tm"], "00000011793144904104");
    }

    #[test]
    fn reply_without_a_token_is_rejected() {
        let error = read_reply(r#"{"ret":401,"msg":"service empty."}"#).unwrap_err();
        assert!(error.to_string().contains("ret 401"), "{error}");
        let error = read_reply(r#"{"ret":200,"code":2000,"msg":"Success","token":""}"#).unwrap_err();
        assert!(error.to_string().contains("without a token"), "{error}");
    }

    #[test]
    fn success_reply_yields_the_vendor_token() {
        let text = format!(r#"{{"ret":200,"code":2000,"msg":"Success","token":"{TOKEN}"}}"#);
        let token = read_reply(&text).unwrap();
        assert_eq!(token.raw, TOKEN);
    }

    #[test]
    fn claims_carry_sn_service_and_media_ip() {
        let text = format!(r#"{{"ret":200,"code":2000,"msg":"Success","token":"{TOKEN}"}}"#);
        let claims = read_reply(&text).unwrap().claims().unwrap();
        assert_eq!(claims.app, "acbe419b46");
        assert_eq!(claims.sn, "ZHA4006PROBE0001");
        assert_eq!(claims.service, "dss");
        assert_eq!(claims.media_ip, "103.85.122.66");
        assert_eq!(claims.tag, "md5x");
        assert_eq!(claims.expires, 1_793_144_904_104);
    }
}
