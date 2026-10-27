use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::client::{CloudError, CloudResult};

pub const STATUS_HOST: &str = "pub-status.secu100.net";
pub const STATUS_PORTS: [u16; 7] = [7700, 7701, 7702, 7703, 7704, 7705, 7706];
pub const STATUS_PATH: &str = "/v1/status/third";
pub const DISCOVERY_RELAY_PORT: u16 = 8765;
pub const DEFAULT_ACCESS_PORT: u16 = 6510;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceOnline {
    pub online: bool,
    pub status: String,
    pub checked_at: u64,
    pub port: u16,
    pub wan_ip: Option<String>,
    pub server_ip: Option<String>,
    pub server_port: Option<u16>,
    pub device_type: Option<String>,
    pub error: Option<String>,
    pub replies: Vec<PortReply>,
    pub servers: Vec<StatusEndpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEndpoint {
    pub port: u16,
    pub status: String,
    pub online: bool,
    pub server_ip: Option<String>,
    pub server_port: Option<u16>,
    pub wan_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortReply {
    pub port: u16,
    pub status: Option<String>,
    pub error: Option<String>,
}

impl DeviceOnline {
    pub fn summary(&self) -> String {
        if let Some(error) = self.error.as_deref() {
            return format!("cloud status unknown: {error}");
        }
        let mut text = format!("cloud status {}", self.status.to_lowercase());
        if let Some(ip) = self.wan_ip.as_deref().filter(|value| !value.is_empty()) {
            text.push_str(&format!(" from {ip}"));
        }
        if let Some(ip) = self.server_ip.as_deref().filter(|value| !value.is_empty()) {
            text.push_str(&format!(" via {ip}"));
            if let Some(port) = self.server_port {
                text.push_str(&format!(":{port}"));
            }
        }
        text.push_str(&format!(" (port {})", self.port));
        let silent = self
            .replies
            .iter()
            .filter(|reply| reply.status.is_none())
            .count();
        if silent > 0 {
            text.push_str(&format!(
                ", {silent} of {} ports did not answer",
                self.replies.len()
            ));
        }
        text
    }
}

pub fn unchecked(summary: impl Into<String>) -> DeviceOnline {
    DeviceOnline {
        online: false,
        status: "Unknown".to_string(),
        checked_at: now_millis(),
        port: 0,
        wan_ip: None,
        server_ip: None,
        server_port: None,
        device_type: None,
        error: Some(summary.into()),
        replies: Vec::new(),
        servers: Vec::new(),
    }
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

pub struct StatusClient {
    host: String,
    ports: Vec<u16>,
    http: reqwest::Client,
}

impl StatusClient {
    pub fn new(host: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("proview-cctv-web/0.3")
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("status http client");
        Self {
            host: host.into(),
            ports: STATUS_PORTS.to_vec(),
            http,
        }
    }

    pub fn with_ports(mut self, ports: Vec<u16>) -> Self {
        self.ports = ports;
        self
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub async fn query(&self, serial: &str, auth_code: &str) -> CloudResult<DeviceOnline> {
        let serial = serial.trim();
        if serial.is_empty() {
            return Err(CloudError::Rejected("a device serial is required".into()));
        }
        let body = json!({
            "StatusProtocol": {
                "Header": {
                    "Version": "1.0",
                    "CSeq": "1",
                    "MessageType": "MSG_STATUS_MULTIQUERY_REQ",
                },
                "Body": [{ "SerialNumber": serial, "AuthCode": auth_code }],
            }
        });
        let mut pending = Vec::with_capacity(self.ports.len());
        for port in self.ports.clone() {
            let http = self.http.clone();
            let host = self.host.clone();
            let body = body.clone();
            pending.push(tokio::spawn(async move {
                let url = format!("http://{host}:{port}{STATUS_PATH}");
                let reply = http
                    .post(url)
                    .header("Accept", "*/*")
                    .header("Content-Type", "application/json")
                    .body(body.to_string())
                    .send()
                    .await;
                match reply {
                    Ok(response) => match response.text().await {
                        Ok(text) => (port, Ok(text)),
                        Err(error) => (port, Err(error.to_string())),
                    },
                    Err(error) => (port, Err(error.to_string())),
                }
            }));
        }

        let mut replies = Vec::with_capacity(pending.len());
        let mut servers = Vec::with_capacity(pending.len());
        let mut best: Option<(u16, StatusEntry)> = None;
        let mut first_status: Option<(u16, StatusEntry)> = None;
        for handle in pending {
            let (port, outcome) = match handle.await {
                Ok(value) => value,
                Err(error) => {
                    replies.push(PortReply {
                        port: 0,
                        status: None,
                        error: Some(error.to_string()),
                    });
                    continue;
                }
            };
            match outcome {
                Ok(text) => match parse_status(&text) {
                    Some(entry) => {
                        replies.push(PortReply {
                            port,
                            status: Some(entry.status.clone()),
                            error: None,
                        });
                        servers.push(StatusEndpoint {
                            port,
                            online: entry.is_online(),
                            status: entry.status.clone(),
                            server_ip: entry.server_ip.clone(),
                            server_port: entry.server_port,
                            wan_ip: entry.wan_ip.clone(),
                        });
                        if entry.is_online() && best.is_none() {
                            best = Some((port, entry));
                        } else if first_status.is_none() {
                            first_status = Some((port, entry));
                        }
                    }
                    None => {
                        let error =
                            parse_error(&text).unwrap_or_else(|| "unreadable reply".to_string());
                        replies.push(PortReply {
                            port,
                            status: None,
                            error: Some(error),
                        });
                    }
                },
                Err(error) => {
                    replies.push(PortReply {
                        port,
                        status: None,
                        error: Some(error),
                    });
                }
            }
        }
        replies.sort_by_key(|reply| reply.port);

        let chosen = best.or(first_status).ok_or_else(|| {
            let detail = replies
                .iter()
                .filter_map(|reply| reply.error.as_deref())
                .next()
                .unwrap_or("no status port answered");
            CloudError::Unreachable(format!("cloud status: {detail}"))
        })?;
        let (port, entry) = chosen;
        Ok(DeviceOnline {
            online: entry.is_online(),
            status: entry.status.clone(),
            checked_at: now_millis(),
            port,
            wan_ip: entry.wan_ip,
            server_ip: entry.server_ip,
            server_port: entry.server_port,
            device_type: entry.device_type,
            error: None,
            replies,
            servers,
        })
    }
}

impl DeviceOnline {
    pub fn access_server(&self) -> Option<(String, u16)> {
        let candidates = self.servers.iter().filter(|entry| entry.online);
        let preferred = candidates
            .clone()
            .find(|entry| entry.server_port != Some(DISCOVERY_RELAY_PORT))
            .or_else(|| candidates.clone().next())?;
        let host = preferred.server_ip.clone()?;
        Some((host, preferred.server_port.unwrap_or(DEFAULT_ACCESS_PORT)))
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct StatusEntry {
    #[serde(rename = "SerialNumber")]
    pub serial_number: String,
    #[serde(rename = "Status")]
    pub status: String,
    #[serde(rename = "WanIP")]
    pub wan_ip: Option<String>,
    #[serde(rename = "ServerIP")]
    pub server_ip: Option<String>,
    #[serde(rename = "ServerPort", deserialize_with = "flexible_port")]
    pub server_port: Option<u16>,
    #[serde(rename = "DeviceType")]
    pub device_type: Option<String>,
}

fn flexible_port<'de, D>(deserializer: D) -> Result<Option<u16>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::Number(number)) => number.as_u64().map(|raw| raw as u16),
        Some(serde_json::Value::String(text)) => text.trim().parse::<u16>().ok(),
        _ => None,
    })
}

impl StatusEntry {
    pub fn is_online(&self) -> bool {
        self.status.eq_ignore_ascii_case("online")
    }
}

pub fn parse_status(text: &str) -> Option<StatusEntry> {
    let parsed: Value = serde_json::from_str(text).ok()?;
    let body = parsed.get("StatusProtocol")?.get("Body")?;
    let first = body.as_array()?.first()?;
    let entry: StatusEntry = serde_json::from_value(first.clone()).ok()?;
    if entry.status.is_empty() {
        return None;
    }
    Some(entry)
}

pub fn parse_error(text: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(text).ok()?;
    let header = parsed.get("StatusProtocol")?.get("Header")?;
    let message = header.get("ErrorString")?.as_str()?.to_string();
    let code = header.get("ErrorNum").and_then(Value::as_str).unwrap_or("");
    if code.is_empty() {
        Some(message)
    } else {
        Some(format!("{message} ({code})"))
    }
}

pub fn auth_code_of(device: &super::types::CloudDevice) -> String {
    for key in ["css", "dss", "tps", "rps", "p2P", "pms"] {
        if let Some(value) = device.extra.get(key).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                return value.to_string();
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONLINE: &str = r#"{"StatusProtocol":{"Body":[{"WanIP":"103.85.122.66","SerialNumber":"7f92970ddf5282743hrc","ServerPort":"8765","Status":"Online","ServerIP":"110.239.85.50","NatType":"0"}],"Header":{"MessageType":"MSG_STATUS_MULTIQUERY_RSP","ErrorNum":"200","ErrorString":"Success OK","Version":"1.0","CSeq":"1"}}}"#;
    const ONLINE_CAMERA: &str = r#"{"StatusProtocol":{"Body":[{"Status":"Online","WanIP":"103.85.122.66","ServerIP":"110.239.87.207","DeviceType":"Camera","SerialNumber":"7f92970ddf5282743hrc","DevicePort":"34567","ServerPort":"6510","KcpEnable":"0"}],"Header":{"Version":"1.0","CSeq":"1","MessageType":"MSG_STATUS_MULTIQUERY_RSP","ErrorNum":"200","ErrorString":"Success OK"}}}"#;
    const OFFLINE: &str = r#"{"StatusProtocol":{"Body":[{"SerialNumber":"7f92970ddf5282743hrc","Status":"Offline"}],"Header":{"MessageType":"MSG_STATUS_MULTIQUERY_RSP","ErrorNum":"200","ErrorString":"success","Version":"1.0","CSeq":"1"}}}"#;
    const REFUSED: &str = r#"{"StatusProtocol":{"Header":{"Version":"1.0","CSeq":"1","MessageType":"MSG_STATUS_MULTIQUERY_RSP","ErrorNum":"400","ErrorString":"all dev ac is invalid"}}}"#;

    #[test]
    fn parses_an_online_reply_with_its_route() {
        let entry = parse_status(ONLINE).expect("status entry");
        assert!(entry.is_online());
        assert_eq!(entry.wan_ip.as_deref(), Some("103.85.122.66"));
        assert_eq!(entry.server_ip.as_deref(), Some("110.239.85.50"));
        assert_eq!(entry.server_port, Some(8765));
        assert_eq!(entry.serial_number, "7f92970ddf5282743hrc");
    }

    #[test]
    fn parses_the_second_cluster_shape() {
        let entry = parse_status(ONLINE_CAMERA).expect("status entry");
        assert!(entry.is_online());
        assert_eq!(entry.device_type.as_deref(), Some("Camera"));
        assert_eq!(entry.server_ip.as_deref(), Some("110.239.87.207"));
        assert_eq!(entry.server_port, Some(6510));
    }

    #[test]
    fn parses_an_offline_reply() {
        let entry = parse_status(OFFLINE).expect("status entry");
        assert!(!entry.is_online());
        assert_eq!(entry.status, "Offline");
    }

    #[test]
    fn reads_the_cluster_refusal() {
        assert!(parse_status(REFUSED).is_none());
        assert_eq!(
            parse_error(REFUSED).as_deref(),
            Some("all dev ac is invalid (400)")
        );
    }

    #[test]
    fn prefers_whichever_port_says_online() {
        let mut online = parse_status(ONLINE).unwrap();
        online.status = "Online".into();
        assert!(online.is_online());
        let offline = parse_status(OFFLINE).unwrap();
        assert!(!offline.is_online());
    }

    fn endpoint(port: u16, host: &str, server_port: u16) -> StatusEndpoint {
        StatusEndpoint {
            port,
            status: "Online".into(),
            online: true,
            server_ip: Some(host.into()),
            server_port: Some(server_port),
            wan_ip: None,
        }
    }

    #[test]
    fn picks_the_access_server_over_the_discovery_relay() {
        let mut status = unchecked("none");
        status.online = true;
        status.servers = vec![
            endpoint(7703, "110.239.85.50", DISCOVERY_RELAY_PORT),
            endpoint(7705, "110.239.87.207", 6510),
        ];
        assert_eq!(
            status.access_server(),
            Some(("110.239.87.207".to_string(), 6510))
        );
    }

    #[test]
    fn falls_back_to_the_discovery_cluster_when_it_is_all_there_is() {
        let mut status = unchecked("none");
        status.online = true;
        status.servers = vec![endpoint(7703, "110.239.85.50", DISCOVERY_RELAY_PORT)];
        assert_eq!(
            status.access_server(),
            Some(("110.239.85.50".to_string(), DISCOVERY_RELAY_PORT))
        );
    }
}
