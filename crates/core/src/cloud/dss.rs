use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::client::{CloudError, CloudResult};
use super::envelope::{signed_request, Identity, Signed};

pub const TOKEN_BASE: &str = "https://pub-token.xmeye.net";
pub const TOKEN_PATH: &str = "/reqToken";
pub const TOKEN_SERVICE: &str = "dss";
pub const PMS_BASE: &str = "http://access-pms.secu100.net:6602";
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
        return Err(CloudError::Malformed(
            "token endpoint answered without a token".into(),
        ));
    }
    Ok(DssToken {
        raw: raw.to_string(),
    })
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

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct CloudRecording {
    pub start: String,
    pub end: String,
    pub duration_secs: u32,
    pub size_bytes: Option<u64>,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct RecordingPage {
    pub recordings: Vec<CloudRecording>,
    pub notice: Option<String>,
}

pub struct RecordingQuery<'a> {
    pub access: &'a str,
    pub account: &'a str,
    pub sn: &'a str,
    pub channel: u32,
    pub day: &'a str,
}

pub fn query_body(request: &RecordingQuery<'_>, auth: &str) -> String {
    json!({
        "msg": "video_query",
        "sn": request.sn,
        "st": format!("{} 00:00:00", request.day),
        "et": format!("{} 23:59:59", request.day),
        "auth": auth,
        "ch": request.channel,
        "muserid": request.account,
    })
    .to_string()
}

pub fn read_recordings(text: &str) -> CloudResult<RecordingPage> {
    let root: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| CloudError::Malformed(format!("{error}: {text}")))?;
    let code = root
        .get("code")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    if code != "200" {
        return Err(CloudError::Rejected(format!(
            "cloud storage answered code {code}: {text}"
        )));
    }
    let body = root
        .get("AlarmCenter")
        .and_then(|value| value.get("Body"))
        .cloned()
        .unwrap_or_else(|| root.clone());
    let mut recordings = Vec::new();
    for key in ["VideoArray", "TimeAxis", "Clips"] {
        if let Some(items) = body.get(key).and_then(|value| value.as_array()) {
            recordings.extend(items.iter().filter_map(item_of));
        }
    }
    recordings.sort_by(|left, right| left.start.cmp(&right.start));
    let notice = recordings.is_empty().then(|| {
        "the cloud answered the query with no clips; an account without an active cloud plan answers exactly this"
            .to_string()
    });
    Ok(RecordingPage { recordings, notice })
}

fn item_of(item: &serde_json::Value) -> Option<CloudRecording> {
    let start = text_field(item, &["BeginTime", "StartTime", "start"])?;
    let end = text_field(item, &["EndTime", "StopTime", "end"]).unwrap_or_default();
    let size = item
        .get("FileLength")
        .or_else(|| item.get("size"))
        .and_then(|value| value.as_u64());
    let path = text_field(item, &["FilePath", "path"]).unwrap_or_default();
    let duration_secs = duration_between(&start, &end).unwrap_or_default();
    Some(CloudRecording {
        start,
        end,
        duration_secs,
        size_bytes: size,
        path,
    })
}

fn text_field(item: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| item.get(*key).and_then(|value| value.as_str()))
        .map(str::to_string)
}

fn duration_between(start: &str, end: &str) -> Option<u32> {
    let begin = stamp_seconds(start)?;
    let stop = stamp_seconds(end)?;
    (stop >= begin).then(|| (stop - begin) as u32)
}

fn stamp_seconds(text: &str) -> Option<i64> {
    let (date, clock) = text.split_once(' ')?;
    let mut numbers = date.split('-').map(str::parse::<i64>);
    let year = numbers.next()?.ok()?;
    let month = numbers.next()?.ok()?;
    let day = numbers.next()?.ok()?;
    let mut parts = clock.split(':').map(str::parse::<i64>);
    let hours = parts.next()?.ok()?;
    let minutes = parts.next()?.ok()?;
    let seconds = parts.next().and_then(|value| value.ok()).unwrap_or(0);
    let shift = if month <= 2 { 1 } else { 0 };
    let years = year - shift;
    let era = years.div_euclid(400);
    let year_of_era = years - era * 400;
    let day_of_year = (153 * (month + (-3 + 12 * shift)) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era;
    Some(days * 86_400 + hours * 3_600 + minutes * 60 + seconds)
}

pub async fn cloud_recordings(request: RecordingQuery<'_>) -> CloudResult<RecordingPage> {
    let identity = Identity::resolve();
    if !identity.is_complete() {
        return Err(CloudError::Malformed(
            super::envelope::MISSING_CLOUD_IDENTITY.to_string(),
        ));
    }
    let client = super::envelope::EnvelopeClient::new(identity);
    let devices = client.device_list(request.access).await?;
    let device = devices
        .iter()
        .find(|device| device.uuid == request.sn)
        .or_else(|| devices.first())
        .ok_or_else(|| CloudError::Rejected("the account has no device to query".to_string()))?;
    let auth = device
        .extra
        .get("pms")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let sn = if request.sn.trim().is_empty() {
        device.uuid.clone()
    } else {
        request.sn.to_string()
    };
    let resolved = RecordingQuery {
        access: request.access,
        account: request.account,
        sn: &sn,
        channel: request.channel,
        day: request.day,
    };
    let body = query_body(&resolved, &auth);
    let http = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECONDS))
        .build()
        .map_err(|error| CloudError::Unreachable(error.to_string()))?;
    let response = http
        .post(format!("{PMS_BASE}/v2"))
        .header("Content-Type", "application/json")
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
            "cloud storage answered {status}: {text}"
        )));
    }
    read_recordings(&text)
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
        let error =
            read_reply(r#"{"ret":200,"code":2000,"msg":"Success","token":""}"#).unwrap_err();
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

    #[test]
    fn query_body_carries_the_day_and_the_device_auth() {
        let request = RecordingQuery {
            access: "a",
            account: "user-1",
            sn: "SN0001",
            channel: 0,
            day: "2026-10-27",
        };
        let parsed: serde_json::Value = serde_json::from_str(&query_body(&request, "abc")).unwrap();
        assert_eq!(parsed["msg"], "video_query");
        assert_eq!(parsed["sn"], "SN0001");
        assert_eq!(parsed["st"], "2026-10-27 00:00:00");
        assert_eq!(parsed["et"], "2026-10-27 23:59:59");
        assert_eq!(parsed["auth"], "abc");
        assert_eq!(parsed["ch"], 0);
        assert_eq!(parsed["muserid"], "user-1");
    }

    #[test]
    fn acknowledgement_without_clips_is_an_empty_page() {
        let page = read_recordings(r#"{"msg":"video_query","code":"200"}"#).unwrap();
        assert!(page.recordings.is_empty());
        assert!(page.notice.unwrap().contains("no clips"));
        let error = read_recordings(r#"{"msg":"video_query","code":"401"}"#).unwrap_err();
        assert!(error.to_string().contains("code 401"), "{error}");
    }

    #[test]
    fn clip_array_maps_to_recordings() {
        let text = r#"{"code":"200","AlarmCenter":{"Body":{"VideoArray":[
            {"BeginTime":"2026-10-27 21:20:00","EndTime":"2026-10-27 21:24:30","FileLength":7340032,"FilePath":"/cloud/a.mp4"},
            {"BeginTime":"2026-10-27 08:00:00","EndTime":"2026-10-27 08:01:00","FileLength":1048576,"FilePath":"/cloud/b.mp4"}
        ]}}}"#;
        let page = read_recordings(text).unwrap();
        assert_eq!(page.recordings.len(), 2);
        assert_eq!(page.recordings[0].start, "2026-10-27 08:00:00");
        assert_eq!(page.recordings[0].duration_secs, 60);
        assert_eq!(page.recordings[1].duration_secs, 270);
        assert_eq!(page.recordings[1].size_bytes, Some(7_340_032));
        assert!(page.notice.is_none());
    }

    #[test]
    fn durations_survive_a_midnight_boundary() {
        assert_eq!(
            duration_between("2026-10-27 23:59:30", "2026-10-28 00:00:30"),
            Some(60)
        );
        assert_eq!(
            duration_between("2026-02-28 23:00:00", "2026-03-01 01:00:00"),
            Some(7200)
        );
        assert_eq!(
            duration_between("2026-10-27 10:00:00", "2026-10-27 09:00:00"),
            None
        );
    }
}
