use serde::{Deserialize, Serialize};

pub const CODE_OK: i32 = 2000;
pub const CODE_OK_LEGACY: i32 = 200;
pub const CODE_CAPTCHA_INVALID: i32 = -1;
pub const CODE_CAPTCHA_EXPIRED: i32 = -2;
pub const CODE_TOKEN_EXPIRED: i32 = -4004;
pub const CODE_BAD_CREDENTIALS: i32 = 4000;
pub const CODE_CAPTCHA_WRONG: i32 = 4010;
pub const CODE_MAIL_UNKNOWN: i32 = 4024;
pub const CODE_USER_UNKNOWN: i32 = 4025;
pub const CODE_CAPTCHA_LOCKED: i32 = 4048;
pub const CODE_TOO_FREQUENT: i32 = 4050;

#[derive(Debug, Clone, Deserialize)]
pub struct Envelope {
    pub code: i32,
    #[serde(default)]
    pub msg: Option<String>,
    #[serde(default)]
    pub data: serde_json::Value,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Envelope {
    pub fn is_ok(&self) -> bool {
        self.code == CODE_OK || self.code == CODE_OK_LEGACY
    }

    pub fn token(&self) -> Option<&str> {
        self.extra
            .get("accessToken")
            .and_then(serde_json::Value::as_str)
    }

    pub fn message(&self) -> String {
        user_message(self.code, self.msg.as_deref())
    }
}

pub fn user_message(code: i32, vendor: Option<&str>) -> String {
    match code {
        CODE_OK | CODE_OK_LEGACY => "ok".to_string(),
        CODE_BAD_CREDENTIALS => "email or password is wrong".to_string(),
        CODE_CAPTCHA_INVALID | CODE_CAPTCHA_WRONG => "verification code is wrong".to_string(),
        CODE_CAPTCHA_EXPIRED => "verification code expired; request a new one".to_string(),
        CODE_CAPTCHA_LOCKED => {
            "too many wrong verification codes; wait before trying again".to_string()
        }
        CODE_TOO_FREQUENT => "the cloud asked to slow down; wait a moment".to_string(),
        CODE_TOKEN_EXPIRED => "session expired; sign in again".to_string(),
        CODE_MAIL_UNKNOWN => {
            "this email is not registered on the cloud account service".to_string()
        }
        CODE_USER_UNKNOWN => {
            "this username is not registered on the cloud account service".to_string()
        }
        other => match vendor.map(str::trim).filter(|text| !text.is_empty()) {
            Some(text) => format!("cloud rejected the call: {text} ({other})"),
            None => format!("cloud rejected the call with code {other}"),
        },
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Captcha {
    pub image: String,
    pub token: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CloudDevice {
    #[serde(default)]
    pub uuid: String,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub channel: Option<u32>,
    #[serde(default, rename = "channelNum")]
    pub channel_num: Option<u32>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl CloudDevice {
    pub fn display_name(&self) -> &str {
        if self.nickname.trim().is_empty() {
            &self.uuid
        } else {
            &self.nickname
        }
    }

    pub fn channel_count(&self) -> u32 {
        self.channel.or(self.channel_num).unwrap_or(1).max(1)
    }

    pub fn to_provider(
        &self,
        status: Option<&super::status::DeviceOnline>,
    ) -> crate::provider::Device {
        crate::provider::Device {
            id: self.uuid.clone(),
            name: self.display_name().to_string(),
            model: self
                .model
                .clone()
                .or_else(|| self.code.clone())
                .unwrap_or_else(|| "xm-cloud".to_string()),
            online: status.map(|value| value.online).unwrap_or(false),
            channels: self.channel_count(),
            lan_host: None,
            online_checked_at: status.map(|value| value.checked_at),
            online_status: status.map(|value| value.status.clone()),
            online_detail: status.map(|value| value.summary()),
        }
    }

    pub fn auth_code(&self) -> String {
        super::status::auth_code_of(self)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceStatus {
    #[serde(default)]
    pub uuid: String,
    #[serde(default)]
    pub status: String,
    #[serde(default, rename = "wakeUpStatus")]
    pub wake_up_status: Option<String>,
}

impl DeviceStatus {
    pub fn online(&self) -> bool {
        self.status == "online"
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CloudProfile {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct CloudSession {
    pub account: String,
    pub access_token: String,
}

impl CloudSession {
    pub fn token(&self) -> &str {
        &self.access_token
    }
}

impl std::fmt::Debug for CloudSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CloudSession")
            .field("account", &self.account)
            .field("access_token", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_codes_to_readable_text() {
        assert_eq!(
            user_message(CODE_BAD_CREDENTIALS, None),
            "email or password is wrong"
        );
        assert!(user_message(CODE_CAPTCHA_LOCKED, None).contains("too many"));
        assert!(user_message(CODE_MAIL_UNKNOWN, None).contains("not registered"));
        assert_eq!(
            user_message(CODE_TOKEN_EXPIRED, None),
            "session expired; sign in again"
        );
    }

    #[test]
    fn accepts_both_success_codes() {
        let legacy: Envelope = serde_json::from_value(serde_json::json!({ "code": 200 })).unwrap();
        let current: Envelope =
            serde_json::from_value(serde_json::json!({ "code": 2000 })).unwrap();
        assert!(legacy.is_ok());
        assert!(current.is_ok());
    }

    #[test]
    fn explains_expired_captcha() {
        assert!(user_message(CODE_CAPTCHA_EXPIRED, None).contains("expired"));
    }

    #[test]
    fn falls_back_to_vendor_message() {
        assert_eq!(
            user_message(4099, Some("boom")),
            "cloud rejected the call: boom (4099)"
        );
        assert_eq!(
            user_message(4099, None),
            "cloud rejected the call with code 4099"
        );
    }

    #[test]
    fn keeps_unknown_device_fields() {
        let raw = serde_json::json!({
            "uuid": "abc123",
            "nickname": "",
            "username": "owner",
            "deviceType": "IPC",
            "wakeUpStatus": "sleep"
        });
        let device: CloudDevice = serde_json::from_value(raw).unwrap();
        assert_eq!(device.display_name(), "abc123");
        assert_eq!(device.channel_count(), 1);
        assert!(device.extra.contains_key("deviceType"));
    }

    #[test]
    fn maps_cloud_device_to_provider_device() {
        let device = CloudDevice {
            uuid: "abc".into(),
            nickname: "Front door".into(),
            code: Some("XM-01".into()),
            channel: Some(2),
            ..CloudDevice::default()
        };
        let status = crate::cloud::status::DeviceOnline {
            online: true,
            status: "Online".into(),
            checked_at: 1,
            port: 7703,
            wan_ip: None,
            server_ip: None,
            server_port: None,
            device_type: None,
            error: None,
            replies: Vec::new(),
        };
        let mapped = device.to_provider(Some(&status));
        assert_eq!(mapped.id, "abc");
        assert_eq!(mapped.name, "Front door");
        assert_eq!(mapped.model, "XM-01");
        assert_eq!(mapped.channels, 2);
        assert!(mapped.online);
    }

    #[test]
    fn redacts_session_token_in_debug() {
        let session = CloudSession {
            account: "a@b.c".into(),
            access_token: "secret-token".into(),
        };
        let text = format!("{session:?}");
        assert!(!text.contains("secret-token"));
        assert!(text.contains("a@b.c"));
    }
}
