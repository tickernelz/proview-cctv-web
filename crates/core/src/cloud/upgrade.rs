use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

pub const DEFAULT_BASE: &str = "https://upgrade.secu100.net:9083";
pub const BASE_ENV: &str = "PROVIEW_UPGRADE_BASE";
pub const DEVICE_TYPES: [&str; 7] = ["DVR", "NVS", "IPC", "HVR", "IVR", "MVR", "IOT"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeQuery {
    pub dev_id: String,
    pub dev_type: String,
    pub cur_version: String,
    pub language: String,
    pub serial: String,
    pub pid: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FirmwareOffer {
    pub dev_id: Option<String>,
    pub date: Option<String>,
    pub file_name: Option<String>,
    pub file_level: Option<Value>,
    pub change_log: Option<Value>,
    pub file_size: Option<Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ServerCheck {
    pub http_status: u16,
    pub up_to_date: bool,
    pub offers: Vec<FirmwareOffer>,
    pub detail: String,
}

pub fn request_body(query: &UpgradeQuery) -> Value {
    let mut body = json!({
        "DevID": query.dev_id,
        "DevType": query.dev_type,
        "CurVersion": query.cur_version,
        "Expect": "Latest",
        "Language": query.language,
        "Manual": "true",
        "SN": query.serial,
        "UUID": "MoblieAPP",
        "FROM": "MoblieAPP",
    });
    if let Some(pid) = query.pid.as_deref().filter(|pid| !pid.is_empty()) {
        body["PID"] = json!(pid);
    }
    body
}

fn text(entry: &Value, key: &str) -> Option<String> {
    entry.get(key).and_then(Value::as_str).map(str::to_string)
}

fn offer(entry: &Value) -> Option<FirmwareOffer> {
    entry.as_object()?;
    Some(FirmwareOffer {
        dev_id: text(entry, "DevID"),
        date: text(entry, "Date").or_else(|| text(entry, "ReleaseDate")),
        file_name: text(entry, "FileName"),
        file_level: entry.get("FileLevel").cloned(),
        change_log: entry.get("ChangeLog").cloned(),
        file_size: entry.get("FileSize").cloned(),
    })
}

pub fn parse_offers(body: &Value) -> Vec<FirmwareOffer> {
    match body {
        Value::Array(entries) => entries.iter().filter_map(offer).collect(),
        Value::Object(_) => offer(body)
            .filter(|found| found.file_name.is_some() || found.date.is_some())
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

pub fn interpret(status: u16, body: &str) -> Result<ServerCheck, String> {
    match status {
        204 => Ok(ServerCheck {
            http_status: status,
            up_to_date: true,
            offers: Vec::new(),
            detail: "the upgrade server offers no newer firmware for this build (HTTP 204)"
                .to_string(),
        }),
        200 => {
            let parsed: Value = serde_json::from_str(body).map_err(|error| {
                format!("the upgrade server answered 200 with a body that is not JSON: {error}")
            })?;
            let offers = parse_offers(&parsed);
            Ok(ServerCheck {
                http_status: status,
                up_to_date: offers.is_empty(),
                detail: if offers.is_empty() {
                    "the upgrade server answered 200 with no firmware entry".to_string()
                } else {
                    format!(
                        "the upgrade server offers {} firmware file(s)",
                        offers.len()
                    )
                },
                offers,
            })
        }
        other => Err(format!("the upgrade server answered HTTP {other}")),
    }
}

pub struct UpgradeClient {
    base: String,
    http: reqwest::Client,
}

impl UpgradeClient {
    pub fn new(base: impl Into<String>, timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("proview-cctv-web/0.1")
            .timeout(timeout)
            .build()
            .expect("upgrade http client");
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            http,
        }
    }

    pub fn from_env(timeout: Duration) -> Self {
        let base = std::env::var(BASE_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE.to_string());
        Self::new(base, timeout)
    }

    pub async fn check(&self, query: &UpgradeQuery) -> Result<ServerCheck, String> {
        let response = self
            .http
            .post(format!("{}/list", self.base))
            .header("Content-Type", "application/json")
            .body(request_body(query).to_string())
            .send()
            .await
            .map_err(|error| format!("the upgrade server did not answer: {error}"))?;
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        interpret(status, &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query() -> UpgradeQuery {
        UpgradeQuery {
            dev_id: "ABCDEF01100101234560000000".to_string(),
            dev_type: "IPC".to_string(),
            cur_version: "2024-05-16".to_string(),
            language: "English".to_string(),
            serial: "serial-one".to_string(),
            pid: None,
        }
    }

    #[test]
    fn body_matches_the_sdk_list_request() {
        assert_eq!(
            request_body(&query()),
            json!({
                "DevID":"ABCDEF01100101234560000000","DevType":"IPC","CurVersion":"2024-05-16",
                "Expect":"Latest","Language":"English","Manual":"true","SN":"serial-one",
                "UUID":"MoblieAPP","FROM":"MoblieAPP"
            })
        );
        assert_eq!(DEVICE_TYPES[2], "IPC");
        let with_pid = UpgradeQuery {
            pid: Some("PID-ONE".to_string()),
            ..query()
        };
        assert_eq!(request_body(&with_pid)["PID"], "PID-ONE");
    }

    #[test]
    fn no_content_means_up_to_date() {
        let check = interpret(204, "").unwrap();
        assert!(check.up_to_date);
        assert!(check.offers.is_empty());
    }

    #[test]
    fn offers_parse_from_object_or_array() {
        let one = interpret(
            200,
            r#"{"DevID":"X","Date":"2025-01-01","FileName":"fw.bin","FileLevel":1,"ChangeLog":"c","FileSize":10}"#,
        )
        .unwrap();
        assert!(!one.up_to_date);
        assert_eq!(one.offers[0].file_name.as_deref(), Some("fw.bin"));
        let many = interpret(200, r#"[{"FileName":"a"},{"FileName":"b"}]"#).unwrap();
        assert_eq!(many.offers.len(), 2);
        assert!(interpret(500, "").is_err());
        assert!(interpret(200, "nope").is_err());
    }

    #[tokio::test]
    async fn client_posts_json_to_list() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 4096];
            let mut seen = Vec::new();
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                seen.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&seen).to_string();
                if let Some(split) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if seen.len() >= split + 4 + length {
                        break;
                    }
                }
            }
            socket
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            String::from_utf8_lossy(&seen).to_string()
        });
        let client = UpgradeClient::new(format!("http://{address}"), Duration::from_secs(4));
        let check = client.check(&query()).await.unwrap();
        assert!(check.up_to_date);
        let request = server.await.unwrap();
        assert!(request.starts_with("POST /list HTTP/1.1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("content-type: application/json"));
        let body = &request[request.find("\r\n\r\n").unwrap() + 4..];
        let parsed: Value = serde_json::from_str(body).unwrap();
        assert_eq!(parsed["Expect"], "Latest");
    }
}
