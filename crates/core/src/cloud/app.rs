use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::client::{CloudError, CloudResult};

pub const DEFAULT_BASE: &str = "https://caps.xmcsrv.net";

pub const REQUESTED_CAPS: [&str; 9] = [
    "xmc.css",
    "xmc.voes",
    "pms.voes",
    "pms.sms",
    "net.cellular",
    "xmc.ais.offduty",
    "xmc.ais.recface",
    "xmc.ais.birds",
    "xmc.ais.timealbum",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppCaps {
    #[serde(skip_serializing)]
    pub ret: i32,
    #[serde(skip_serializing)]
    pub msg: String,
    pub sn: String,
    #[serde(rename = "mfrsOemId")]
    pub mfrs_oem_id: String,
    #[serde(rename = "forceOemId")]
    pub force_oem_id: String,
    #[serde(rename = "bossOemId")]
    pub boss_oem_id: String,
    #[serde(rename = "factoryOemId")]
    pub factory_oem_id: String,
    pub last_wan_ip: String,
    pub model: String,
    pub hw: String,
    pub pid: String,
    pub sw: String,
    pub sw_ver: String,
    pub pwd: String,
    pub caps: Value,
}

impl AppCaps {
    pub fn is_known(&self) -> bool {
        !self.model.is_empty() || !self.pid.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppFlags {
    #[serde(skip_serializing)]
    pub ret: i32,
    #[serde(skip_serializing)]
    pub msg: String,
    pub stat: Value,
}

pub struct CapsClient {
    base: String,
    http: reqwest::Client,
    ua: String,
}

impl CapsClient {
    pub fn new(base: impl Into<String>) -> Self {
        let base = base.into().trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .user_agent("proview-cctv-web/0.1")
            .timeout(Duration::from_secs(20))
            .build()
            .expect("caps http client");
        Self {
            base,
            http,
            ua: "ProView/1.0.2".to_string(),
        }
    }

    pub fn from_env() -> Self {
        let base = std::env::var("PROVIEW_CAPS_BASE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE.to_string());
        Self::new(base)
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub async fn req_caps(&self, sn: &str, caps: &[&str]) -> CloudResult<AppCaps> {
        let sn = sn.trim();
        if sn.is_empty() {
            return Err(CloudError::Rejected("a device serial is required".into()));
        }
        let body = json!({
            "method": "reqCaps",
            "body": { "caps": caps, "sn": sn },
        });
        let url = format!("{}/api/asMsgQ", self.base);
        let response = self
            .http
            .post(url)
            .header("User-Agent", self.ua.as_str())
            .json(&body)
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let parsed: AppCaps = serde_json::from_str(&text).map_err(|_| {
            CloudError::Malformed(format!(
                "HTTP {status}: {}",
                text.chars().take(120).collect::<String>()
            ))
        })?;
        if parsed.ret != 200 {
            return Err(CloudError::Rejected(format!(
                "{} ({})",
                parsed.msg, parsed.ret
            )));
        }
        Ok(parsed)
    }

    pub async fn app_caps(&self, pkg: &str) -> CloudResult<AppFlags> {
        let url = format!("{}/api/appCaps?pkg={}", self.base, pkg);
        let response = self
            .http
            .get(url)
            .header("User-Agent", self.ua.as_str())
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let parsed: AppFlags = serde_json::from_str(&text).map_err(|_| {
            CloudError::Malformed(format!(
                "HTTP {status}: {}",
                text.chars().take(120).collect::<String>()
            ))
        })?;
        if parsed.ret != 200 {
            return Err(CloudError::Rejected(format!(
                "{} ({})",
                parsed.msg, parsed.ret
            )));
        }
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPS_REPLY: &str = r#"{"ret":200,"msg":"Success","method":"reqCaps","sn":"AASN000000000001","mfrsOemId":"G51","forceOemId":"G51","bossOemId":"G51","factoryOemId":"G51","lastWanIp":"103.85.122.66","productAttrs":[],"model":"XM535_X6E-WEQ_8M","hw":"A90A007F3000000C","pid":"A90A007F3000000C","sw":"V5.01.R02.G51A07F3.10010.346732.0000000","swVer":"2024-11-01 19:21:23","pwd":"47.86.110.52","caps":{"xmc.css.24h.enable":false,"xmc.css.vid.keepdays":0}}"#;

    const FLAGS_REPLY: &str = r#"{"ret":200,"msg":"Success","stat":{"cee":true,"disk":false,"pkg":"com.szyarsor.proviewcctv","vvt":600}}"#;

    #[test]
    fn reads_the_reply_the_app_receives() {
        let caps: AppCaps = serde_json::from_str(CAPS_REPLY).expect("captured reply");
        assert_eq!(caps.ret, 200);
        assert_eq!(caps.sn, "AASN000000000001");
        assert_eq!(caps.model, "XM535_X6E-WEQ_8M");
        assert_eq!(caps.pid, "A90A007F3000000C");
        assert_eq!(caps.last_wan_ip, "103.85.122.66");
        assert_eq!(caps.caps["xmc.css.vid.keepdays"], 0);
    }

    #[test]
    fn reads_the_feature_flags() {
        let flags: AppFlags = serde_json::from_str(FLAGS_REPLY).expect("captured reply");
        assert_eq!(flags.ret, 200);
        assert_eq!(flags.stat["disk"], false);
    }

    #[test]
    fn an_unknown_serial_is_not_a_device() {
        let caps: AppCaps = serde_json::from_str(
            r#"{"ret":200,"msg":"Success","sn":"00000000000000000000","model":"","pid":"","caps":{}}"#,
        )
        .expect("reply for an unknown serial");
        assert_eq!(caps.ret, 200);
        assert!(!caps.is_known());
    }

    #[tokio::test]
    async fn refuses_an_empty_serial() {
        let client = CapsClient::new("http://127.0.0.1:9");
        let error = client
            .req_caps("  ", &["xmc.css"])
            .await
            .expect_err("empty");
        assert!(matches!(error, CloudError::Rejected(_)));
    }
}
