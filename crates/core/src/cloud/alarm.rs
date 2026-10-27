use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::client::{CloudError, CloudResult};

pub const ALARM_HOST: &str = "access-alc.secu100.net";
pub const ALARM_PORT: u16 = 6602;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);
pub const PAGE_LIMIT: u32 = 60;
pub const API_VERSION: &str = "1.0";
pub const TERMINAL: &str = "Camera";
pub const QUERY_BY_TIME: &str = "MSG_ALARM_QUERY_BY_TIME_REQ";
pub const QUERY_PAGE: &str = "MSG_ALARM_QUERY_REQ";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlarmQuery {
    pub serial: String,
    pub auth_code: String,
    #[serde(default)]
    pub channel: u32,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub end: String,
    #[serde(default)]
    pub offset: u32,
    #[serde(default = "default_number")]
    pub number: u32,
}

fn default_number() -> u32 {
    PAGE_LIMIT
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlarmPicture {
    pub object: String,
    #[serde(default)]
    pub bucket: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlarmEntry {
    pub id: String,
    pub event: String,
    #[serde(default)]
    pub event_name: String,
    #[serde(default)]
    pub name: String,
    pub time: String,
    #[serde(default)]
    pub utc_time: i64,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub picture: Option<AlarmPicture>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlarmPage {
    pub serial: String,
    pub total: u32,
    pub finished: bool,
    pub entries: Vec<AlarmEntry>,
    pub queried_at: u64,
    pub latency_ms: u64,
}

#[derive(Clone)]
pub struct AlarmClient {
    host: String,
    port: u16,
    http: reqwest::Client,
}

impl AlarmClient {
    pub fn new(host: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("proview-cctv-web/0.3")
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("alarm http client");
        Self {
            host: host.into(),
            port: ALARM_PORT,
            http,
        }
    }

    pub fn from_env() -> Self {
        let host = std::env::var("PROVIEW_ALARM_HOST")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| ALARM_HOST.to_string());
        Self::new(host)
    }

    pub fn endpoint(&self) -> String {
        format!("http://{}:{}/", self.host, self.port)
    }

    pub async fn list(&self, query: &AlarmQuery) -> CloudResult<AlarmPage> {
        let started = Instant::now();
        let body = request_body(query);
        let reply = self
            .http
            .post(self.endpoint())
            .header("Accept", "*/*")
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.without_url().to_string()))?;
        let status = reply.status();
        let text = reply
            .text()
            .await
            .map_err(|error| CloudError::Unreachable(error.without_url().to_string()))?;
        let mut page = parse_page(&text)?;
        page.serial = if page.serial.is_empty() {
            query.serial.clone()
        } else {
            page.serial
        };
        page.queried_at = now_millis();
        page.latency_ms = started.elapsed().as_millis() as u64;
        if !status.is_success() && page.entries.is_empty() {
            return Err(CloudError::Unreachable(format!(
                "alarm center answered {status}"
            )));
        }
        Ok(page)
    }

}

pub fn request_body(query: &AlarmQuery) -> Value {
    let timed = !query.start.trim().is_empty() && !query.end.trim().is_empty();
    let mut body = json!({
        "SerialNumber": query.serial,
        "AuthCode": query.auth_code,
        "Channel": query.channel,
    });
    let (message, fields) = if timed {
        (
            QUERY_BY_TIME,
            json!({ "StartTime": query.start, "EndTime": query.end }),
        )
    } else {
        (
            QUERY_PAGE,
            json!({ "AlarmOffset": query.offset, "AlarmNumber": query.number }),
        )
    };
    let target = body.as_object_mut().expect("alarm body object");
    for (key, value) in fields.as_object().expect("alarm fields object") {
        target.insert(key.clone(), value.clone());
    }
    json!({
        "AlarmCenter": {
            "Header": {
                "Version": API_VERSION,
                "TerminalType": TERMINAL,
                "CSeq": "1",
                "MessageType": message,
            },
            "Body": body,
        }
    })
}

pub fn parse_page(text: &str) -> CloudResult<AlarmPage> {
    let root: Value = serde_json::from_str(text)
        .map_err(|error| CloudError::Malformed(format!("alarm reply: {error}")))?;
    let center = root
        .get("AlarmCenter")
        .ok_or_else(|| CloudError::Malformed("alarm reply has no AlarmCenter".to_string()))?;
    if let Some(header) = center.get("Header") {
        let code = header
            .get("ErrorNum")
            .and_then(value_text)
            .unwrap_or_default();
        if !code.is_empty() && code != "200" && code != "0" {
            let reason = header
                .get("ErrorString")
                .and_then(Value::as_str)
                .unwrap_or("the alarm center rejected the query");
            return Err(CloudError::Rejected(format!("{reason} ({code})")));
        }
    }
    let body = center
        .get("Body")
        .ok_or_else(|| CloudError::Malformed("alarm reply has no Body".to_string()))?;
    let serial = body
        .get("SerialNumber")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let total = body
        .get("AlarmTotal")
        .and_then(value_u32)
        .unwrap_or_default();
    let finished = body
        .get("IsFinished")
        .and_then(value_text)
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    let entries = body
        .get("AlarmArray")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(entry_of).collect())
        .unwrap_or_default();
    Ok(AlarmPage {
        serial,
        total,
        finished,
        entries,
        queried_at: 0,
        latency_ms: 0,
    })
}

fn entry_of(item: &Value) -> Option<AlarmEntry> {
    let id = item
        .get("AlarmId")
        .and_then(value_text)
        .or_else(|| item.get("AlarmID").and_then(value_text))
        .unwrap_or_default();
    let event = item
        .get("AlarmEvent")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if id.is_empty() && event.is_empty() {
        return None;
    }
    let picture = item.get("PicInfo").and_then(|info| {
        let object = info.get("ObjName").and_then(Value::as_str)?;
        if object.trim().is_empty() {
            return None;
        }
        Some(AlarmPicture {
            object: object.to_string(),
            bucket: info
                .get("StorageBucket")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            size: info.get("ObjSize").and_then(value_u64).unwrap_or_default(),
        })
    });
    Some(AlarmEntry {
        id,
        event: event.clone(),
        event_name: item
            .get("AlarmEventName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        name: event_name(&event),
        time: item
            .get("AlarmTime")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        utc_time: item.get("UtcTime").and_then(value_i64).unwrap_or_default(),
        channel: item.get("Channel").and_then(value_text).unwrap_or_default(),
        message: item
            .get("AlarmMsg")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        status: item
            .get("Status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        device_name: item
            .get("DevName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        picture,
    })
}

pub fn event_name(event: &str) -> String {
    let base = event
        .split(':')
        .next()
        .unwrap_or(event)
        .trim_start_matches("appEvent");
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for letter in base.chars() {
        if letter.is_uppercase() && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.push(letter);
    }
    if !current.is_empty() {
        words.push(current);
    }
    if words.is_empty() {
        return if base.trim().is_empty() {
            "alarm".to_string()
        } else {
            base.to_string()
        };
    }
    words
        .iter()
        .map(|word| {
            let lower = word.to_lowercase();
            let mut letters = lower.chars();
            match letters.next() {
                Some(first) => first.to_uppercase().collect::<String>() + letters.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag {
            "1".to_string()
        } else {
            "0".to_string()
        }),
        _ => None,
    }
}

fn value_u32(value: &Value) -> Option<u32> {
    value_text(value).and_then(|text| text.trim().parse().ok())
}

fn value_u64(value: &Value) -> Option<u64> {
    value_text(value).and_then(|text| text.trim().parse().ok())
}

fn value_i64(value: &Value) -> Option<i64> {
    value_text(value).and_then(|text| text.trim().parse().ok())
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str = r#"{"AlarmCenter":{"Body":{"SerialNumber":"DEV0001","AlarmArray":[{"AlarmEvent":"appEventHumanDetectAlarm:2","AlarmTime":"2026-10-27 20:33:59","PicInfo":{"ObjName":"003_DEV0001_261027203358.jpeg","StorageBucket":"OSS_bucket","ObjSize":66114},"Channel":"0","UtcTime":1793108041,"AlarmMsg":"","AlarmId":"261027203358"},{"AlarmEvent":"appEventMotionDetect:1","AlarmTime":"2026-10-27 17:10:05","Channel":"1","UtcTime":1793095807,"AlarmMsg":"motion","AlarmId":"261027171005"}],"AlarmTotal":2,"IsFinished":"1"},"Header":{"Version":"1.0","CSeq":"1","MessageType":"MSG_ALARM_QUERY_RSP","ErrorNum":"200","ErrorString":"Success OK"}}}"#;

    #[test]
    fn reads_a_page_with_times_and_pictures() {
        let page = parse_page(REPLY).expect("page");
        assert_eq!(page.serial, "DEV0001");
        assert_eq!(page.total, 2);
        assert!(page.finished);
        assert_eq!(page.entries.len(), 2);
        let first = &page.entries[0];
        assert_eq!(first.id, "261027203358");
        assert_eq!(first.time, "2026-10-27 20:33:59");
        assert_eq!(first.event, "appEventHumanDetectAlarm:2");
        assert_eq!(first.name, "Human Detect Alarm");
        assert_eq!(first.channel, "0");
        let picture = first.picture.as_ref().expect("picture");
        assert_eq!(picture.object, "003_DEV0001_261027203358.jpeg");
        assert_eq!(picture.size, 66114);
        assert!(page.entries[1].picture.is_none());
    }

    #[test]
    fn a_refusal_carries_the_vendor_reason() {
        let text = r#"{"AlarmCenter":{"Header":{"MessageType":"MSG_ALARM_QUERY_RSP","ErrorNum":"400","ErrorString":"check authcode failed"}}}"#;
        match parse_page(text) {
            Err(CloudError::Rejected(reason)) => {
                assert!(reason.contains("check authcode failed"), "{reason}");
                assert!(reason.contains("400"), "{reason}");
            }
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[test]
    fn a_timed_query_sends_the_window_and_a_paged_one_sends_the_slice() {
        let timed = request_body(&AlarmQuery {
            serial: "DEV0001".into(),
            auth_code: "code".into(),
            start: "2026-10-01 00:00:00".into(),
            end: "2026-10-27 23:59:59".into(),
            ..Default::default()
        });
        let header = &timed["AlarmCenter"]["Header"];
        assert_eq!(header["MessageType"], QUERY_BY_TIME);
        assert_eq!(header["TerminalType"], "Camera");
        let body = &timed["AlarmCenter"]["Body"];
        assert_eq!(body["SerialNumber"], "DEV0001");
        assert_eq!(body["AuthCode"], "code");
        assert_eq!(body["StartTime"], "2026-10-01 00:00:00");
        assert_eq!(body["EndTime"], "2026-10-27 23:59:59");
        assert!(body.get("AlarmOffset").is_none());

        let paged = request_body(&AlarmQuery {
            serial: "DEV0001".into(),
            auth_code: "code".into(),
            offset: 20,
            number: 10,
            ..Default::default()
        });
        assert_eq!(paged["AlarmCenter"]["Header"]["MessageType"], QUERY_PAGE);
        assert_eq!(paged["AlarmCenter"]["Body"]["AlarmOffset"], 20);
        assert_eq!(paged["AlarmCenter"]["Body"]["AlarmNumber"], 10);
    }

    #[test]
    fn a_serialised_page_never_carries_the_auth_code() {
        let mut page = parse_page(REPLY).expect("page");
        page.serial = "DEV0001".into();
        let json = serde_json::to_string(&page).expect("page json");
        assert!(!json.contains("authCode"), "{json}");
        assert!(!json.contains("/download/"), "{json}");
    }

}
