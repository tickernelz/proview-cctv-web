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
pub const DELETE_REQ: &str = "MSG_ALARM_DELETE_REQ";
pub const CALENDAR_REQ: &str = "MSG_VIDEO_NODE_QUERY_REQ";
pub const SUBSCRIPTION_REQ: &str = "MSG_QUERY_SUBSCRIBE_REQ";
pub const DELETE_TYPE: &str = "MSG";
pub const CALENDAR_TYPE: &str = "MSG";
pub const CALENDAR_STREAM: &str = "Main";
pub const SUBSCRIPTION_ALL: &str = "ALL";
pub const EVENT_FILTERS: [(&str, &str); 2] = [
    ("VideoMotion", "Motion detection"),
    ("appEventHumanDetectAlarm", "Human detection"),
];

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
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub user_id: String,
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
        let port = std::env::var("PROVIEW_ALARM_PORT")
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(ALARM_PORT);
        let mut client = Self::new(host);
        client.port = port;
        client
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn endpoint(&self) -> String {
        format!("http://{}:{}/", self.host, self.port)
    }

    async fn post(&self, body: &Value) -> CloudResult<(reqwest::StatusCode, String)> {
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
        Ok((status, text))
    }

    pub async fn delete(&self, target: &AlarmTarget, scope: &DeleteScope) -> CloudResult<()> {
        let body = delete_body(target, scope)?;
        let (status, text) = self.post(&body).await?;
        center_of(&text, status)?;
        Ok(())
    }

    pub async fn calendar(
        &self,
        target: &AlarmTarget,
        month: &str,
        channel: u32,
    ) -> CloudResult<Vec<AlarmDay>> {
        let body = calendar_body(target, month, channel)?;
        let (status, text) = self.post(&body).await?;
        parse_calendar(&text, status)
    }

    pub async fn subscription(
        &self,
        target: &AlarmTarget,
        tokens: &[String],
    ) -> CloudResult<Vec<String>> {
        let body = subscription_body(target, tokens);
        let (status, text) = self.post(&body).await?;
        parse_subscription(&text, status)
    }

    pub async fn list(&self, query: &AlarmQuery) -> CloudResult<AlarmPage> {
        let started = Instant::now();
        let body = request_body(query);
        let (status, text) = self.post(&body).await?;
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
    if !query.event.trim().is_empty() {
        target.insert("AlarmEvent".to_string(), json!(query.event.trim()));
    }
    if !query.user_id.trim().is_empty() {
        target.insert("UserId".to_string(), json!(query.user_id.trim()));
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlarmTarget {
    pub serial: String,
    pub auth_code: String,
    pub user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteScope {
    Ids(Vec<String>),
    All,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlarmDay {
    pub day: String,
    pub count: u32,
}

pub fn alarm_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub fn month_ok(month: &str) -> bool {
    let bytes = month.as_bytes();
    bytes.len() == 7
        && bytes[4] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || byte.is_ascii_digit())
        && matches!(month[5..].parse::<u32>(), Ok(1..=12))
}

fn envelope(message: &str, body: Value) -> Value {
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

fn target_fields(target: &AlarmTarget) -> serde_json::Map<String, Value> {
    let mut fields = serde_json::Map::new();
    fields.insert("SerialNumber".to_string(), json!(target.serial));
    fields.insert("AuthCode".to_string(), json!(target.auth_code));
    if !target.user_id.trim().is_empty() {
        fields.insert("UserId".to_string(), json!(target.user_id.trim()));
    }
    fields
}

pub fn delete_body(target: &AlarmTarget, scope: &DeleteScope) -> CloudResult<Value> {
    let mut fields = target_fields(target);
    fields.insert("DelType".to_string(), json!(DELETE_TYPE));
    if let DeleteScope::Ids(ids) = scope {
        if ids.is_empty() {
            return Err(CloudError::Rejected(
                "no alarm was selected; clearing every alarm is a separate request".to_string(),
            ));
        }
        if let Some(bad) = ids.iter().find(|id| !alarm_id_ok(id)) {
            return Err(CloudError::Rejected(format!("{bad:?} is not an alarm id")));
        }
        let items: Vec<Value> = ids.iter().map(|id| json!({ "id": id })).collect();
        fields.insert("AlarmId".to_string(), Value::Array(items));
    }
    Ok(envelope(DELETE_REQ, Value::Object(fields)))
}

pub fn calendar_body(target: &AlarmTarget, month: &str, channel: u32) -> CloudResult<Value> {
    if !month_ok(month) {
        return Err(CloudError::Rejected(format!(
            "{month:?} is not a month in YYYY-MM form"
        )));
    }
    let mut fields = target_fields(target);
    fields.insert("Date".to_string(), json!(month));
    fields.insert("StramType".to_string(), json!(CALENDAR_STREAM));
    fields.insert("Type".to_string(), json!(CALENDAR_TYPE));
    fields.insert("Channel".to_string(), json!(channel.to_string()));
    Ok(envelope(CALENDAR_REQ, Value::Object(fields)))
}

pub fn subscription_body(target: &AlarmTarget, tokens: &[String]) -> Value {
    let mut fields = target_fields(target);
    let tokens: Vec<&str> = tokens
        .iter()
        .map(|token| token.trim())
        .filter(|token| !token.is_empty())
        .collect();
    if !tokens.is_empty() {
        fields.insert("Token".to_string(), json!(tokens));
    }
    fields.insert("SubType".to_string(), json!(SUBSCRIPTION_ALL));
    envelope(SUBSCRIPTION_REQ, Value::Object(fields))
}

fn center_of(text: &str, status: reqwest::StatusCode) -> CloudResult<Value> {
    let root: Value = match serde_json::from_str(text) {
        Ok(root) => root,
        Err(error) if status.is_success() => {
            return Err(CloudError::Malformed(format!("alarm reply: {error}")))
        }
        Err(_) => {
            return Err(CloudError::Unreachable(format!(
                "alarm center answered {status}"
            )))
        }
    };
    let center = root
        .get("AlarmCenter")
        .cloned()
        .ok_or_else(|| CloudError::Malformed("alarm reply has no AlarmCenter".to_string()))?;
    let code = center
        .get("Header")
        .and_then(|header| header.get("ErrorNum"))
        .and_then(value_text)
        .unwrap_or_default();
    if code != "200" && code != "0" {
        let reason = center
            .get("Header")
            .and_then(|header| header.get("ErrorString"))
            .and_then(Value::as_str)
            .unwrap_or("the alarm center refused the request");
        return Err(CloudError::Rejected(format!(
            "{reason} ({})",
            if code.is_empty() { "no code" } else { &code }
        )));
    }
    Ok(center)
}

pub fn parse_calendar(text: &str, status: reqwest::StatusCode) -> CloudResult<Vec<AlarmDay>> {
    let center = center_of(text, status)?;
    let mut days: Vec<AlarmDay> = center
        .get("Body")
        .and_then(|body| body.get("Date"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let day = item.get("Time").and_then(Value::as_str)?.trim().to_string();
                    if day.len() != 10 {
                        return None;
                    }
                    let count = item.get("Count").and_then(value_u32).unwrap_or(1);
                    Some(AlarmDay { day, count })
                })
                .collect()
        })
        .unwrap_or_default();
    days.sort_by(|left, right| left.day.cmp(&right.day));
    days.dedup_by(|later, earlier| later.day == earlier.day);
    Ok(days)
}

pub fn parse_subscription(text: &str, status: reqwest::StatusCode) -> CloudResult<Vec<String>> {
    let center = center_of(text, status)?;
    let mut tokens: Vec<String> = Vec::new();
    let rows = match center.get("Body") {
        Some(Value::Array(rows)) => rows.clone(),
        Some(Value::Object(row)) if !row.is_empty() => vec![Value::Object(row.clone())],
        _ => Vec::new(),
    };
    for row in rows {
        if let Some(items) = row.get("Token").and_then(Value::as_array) {
            for item in items {
                if let Some(token) = item.as_str() {
                    if !token.trim().is_empty() && !tokens.iter().any(|seen| seen == token) {
                        tokens.push(token.to_string());
                    }
                }
            }
        }
    }
    Ok(tokens)
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

    fn target() -> AlarmTarget {
        AlarmTarget {
            serial: "serial-one".into(),
            auth_code: "code".into(),
            user_id: "user-one".into(),
        }
    }

    #[test]
    fn deleting_selected_alarms_lists_each_id_under_alarm_id() {
        let body = delete_body(
            &target(),
            &DeleteScope::Ids(vec!["261028205437".into(), "261028205333".into()]),
        )
        .expect("body");
        assert_eq!(body["AlarmCenter"]["Header"]["MessageType"], DELETE_REQ);
        assert_eq!(body["AlarmCenter"]["Header"]["TerminalType"], "Camera");
        let fields = &body["AlarmCenter"]["Body"];
        assert_eq!(fields["SerialNumber"], "serial-one");
        assert_eq!(fields["AuthCode"], "code");
        assert_eq!(fields["UserId"], "user-one");
        assert_eq!(fields["DelType"], "MSG");
        assert_eq!(
            fields["AlarmId"],
            json!([{ "id": "261028205437" }, { "id": "261028205333" }])
        );
    }

    #[test]
    fn clearing_all_alarms_sends_no_id_list_and_an_empty_selection_is_refused() {
        let all = delete_body(&target(), &DeleteScope::All).expect("body");
        assert!(all["AlarmCenter"]["Body"].get("AlarmId").is_none());
        assert_eq!(all["AlarmCenter"]["Body"]["DelType"], "MSG");
        assert!(matches!(
            delete_body(&target(), &DeleteScope::Ids(Vec::new())),
            Err(CloudError::Rejected(_))
        ));
        assert!(matches!(
            delete_body(&target(), &DeleteScope::Ids(vec!["1;2".into()])),
            Err(CloudError::Rejected(_))
        ));
    }

    #[test]
    fn the_month_calendar_asks_for_message_days_on_the_main_stream() {
        let body = calendar_body(&target(), "2026-10", 0).expect("body");
        assert_eq!(body["AlarmCenter"]["Header"]["MessageType"], CALENDAR_REQ);
        let fields = &body["AlarmCenter"]["Body"];
        assert_eq!(fields["Date"], "2026-10");
        assert_eq!(fields["Type"], "MSG");
        assert_eq!(fields["StramType"], "Main");
        assert_eq!(fields["Channel"], "0");
        for bad in ["2026-13", "2026-1", "2026/10", "202610", "2026-00"] {
            assert!(calendar_body(&target(), bad, 0).is_err(), "{bad}");
        }
    }

    #[test]
    fn calendar_days_read_from_an_array_and_an_empty_month_reads_as_none() {
        let ok = reqwest::StatusCode::OK;
        let text = r#"{"AlarmCenter":{"Header":{"ErrorNum":"200"},"Body":{"Date":[{"Time":"2026-10-28","Count":"14"},{"Time":"2026-10-27","Count":"52"}]}}}"#;
        let days = parse_calendar(text, ok).expect("days");
        assert_eq!(
            days,
            vec![
                AlarmDay {
                    day: "2026-10-27".into(),
                    count: 52
                },
                AlarmDay {
                    day: "2026-10-28".into(),
                    count: 14
                },
            ]
        );
        let empty = r#"{"AlarmCenter":{"Header":{"ErrorNum":"200"},"Body":{"Date":{}}}}"#;
        assert!(parse_calendar(empty, ok).expect("empty").is_empty());
        let refused =
            r#"{"AlarmCenter":{"Header":{"ErrorNum":"400","ErrorString":"invalid parameter"}}}"#;
        match parse_calendar(refused, ok) {
            Err(CloudError::Rejected(reason)) => {
                assert!(reason.contains("invalid parameter (400)"))
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_subscription_query_lists_tokens_only_when_given_and_reads_subscribed_ones() {
        let bare = subscription_body(&target(), &[]);
        assert!(bare["AlarmCenter"]["Body"].get("Token").is_none());
        assert_eq!(bare["AlarmCenter"]["Body"]["SubType"], "ALL");
        assert_eq!(
            bare["AlarmCenter"]["Header"]["MessageType"],
            SUBSCRIPTION_REQ
        );
        let named = subscription_body(&target(), &[" a ".into(), "".into()]);
        assert_eq!(named["AlarmCenter"]["Body"]["Token"], json!(["a"]));
        let ok = reqwest::StatusCode::OK;
        let rows = r#"{"AlarmCenter":{"Header":{"ErrorNum":"200"},"Body":[{"SerialNumber":"serial-one","Token":["a","b","a"]}]}}"#;
        assert_eq!(parse_subscription(rows, ok).expect("rows"), vec!["a", "b"]);
        let none = r#"{"AlarmCenter":{"Header":{"ErrorNum":"200"},"Body":{}}}"#;
        assert!(parse_subscription(none, ok).expect("none").is_empty());
    }

    #[test]
    fn an_event_filter_and_the_user_ride_in_the_query_body() {
        let body = request_body(&AlarmQuery {
            serial: "serial-one".into(),
            auth_code: "code".into(),
            start: "2026-10-01 00:00:00".into(),
            end: "2026-10-30 23:59:59".into(),
            event: "appEventHumanDetectAlarm".into(),
            user_id: "user-one".into(),
            ..Default::default()
        });
        assert_eq!(
            body["AlarmCenter"]["Body"]["AlarmEvent"],
            "appEventHumanDetectAlarm"
        );
        assert_eq!(body["AlarmCenter"]["Body"]["UserId"], "user-one");
        let plain = request_body(&AlarmQuery {
            serial: "serial-one".into(),
            auth_code: "code".into(),
            ..Default::default()
        });
        assert!(plain["AlarmCenter"]["Body"].get("AlarmEvent").is_none());
        assert!(plain["AlarmCenter"]["Body"].get("UserId").is_none());
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
