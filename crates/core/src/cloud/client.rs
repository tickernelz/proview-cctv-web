use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use super::digest::digest_of;
use super::types::{
    user_message, Captcha, CloudDevice, CloudProfile, CloudSession, DeviceStatus, Envelope,
    CODE_BAD_CREDENTIALS, CODE_TOKEN_EXPIRED,
};

#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("{0}")]
    Rejected(String),
    #[error("email or password is wrong")]
    Credentials,
    #[error("session expired; sign in again")]
    Expired,
    #[error("cloud is not reachable: {0}")]
    Unreachable(String),
    #[error("cloud replied with an unexpected body: {0}")]
    Malformed(String),
}

pub type CloudResult<T> = Result<T, CloudError>;

pub struct CloudClient {
    base: String,
    http: reqwest::Client,
}

impl CloudClient {
    pub const DEFAULT_BASE: &'static str = "https://api.xmeye.net";

    pub fn new(base: impl Into<String>) -> Self {
        let base = base.into().trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .user_agent("proview-cctv-web/0.1")
            .timeout(Duration::from_secs(20))
            .build()
            .expect("cloud http client");
        Self { base, http }
    }

    pub fn from_env() -> Self {
        let base = std::env::var("PROVIEW_PORTAL_BASE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| Self::DEFAULT_BASE.to_string());
        Self::new(base)
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    async fn call(
        &self,
        path: &str,
        body: serde_json::Value,
        token: Option<&str>,
        code_token: Option<&str>,
    ) -> CloudResult<Envelope> {
        let url = format!("{}{}", self.base, path);
        let mut request = self
            .http
            .post(url)
            .header("Accept", "application/json, text/plain, */*")
            .json(&body);
        if let Some(token) = token {
            request = request.header("Access-Token", token);
        }
        if let Some(code_token) = code_token {
            request = request.header("Code-Token", code_token);
        }
        let response = request
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        match serde_json::from_str::<Envelope>(&text) {
            Ok(envelope) => Ok(envelope),
            Err(_) => Err(CloudError::Malformed(format!(
                "HTTP {status}: {}",
                text.chars().take(120).collect::<String>()
            ))),
        }
    }

    pub async fn captcha(&self) -> CloudResult<Captcha> {
        let envelope = self
            .call("/api/transfer/image/v1/", json!({}), None, None)
            .await?;
        parse_captcha(guard(envelope)?)
    }

    pub async fn login(
        &self,
        account: &str,
        password: &str,
        verification_code: &str,
        code_token: &str,
    ) -> CloudResult<CloudSession> {
        let body = json!({
            "uname": digest_of(account),
            "upass": digest_of(password),
            "verCode": verification_code,
        });
        let envelope = self
            .call("/api/transfer/login/v1", body, None, Some(code_token))
            .await?;
        let envelope = guard(envelope)?;
        let token = envelope
            .token()
            .or_else(|| {
                envelope
                    .data
                    .get("accessToken")
                    .and_then(serde_json::Value::as_str)
            })
            .ok_or_else(|| {
                CloudError::Malformed("login reported success without an access token".to_string())
            })?;
        Ok(CloudSession {
            account: account.to_string(),
            access_token: token.to_string(),
        })
    }

    pub async fn profile(&self, session: &CloudSession) -> CloudResult<CloudProfile> {
        let body = json!({ "Authorization": session.token() });
        let envelope = self
            .call(
                "/api/transfer/userinfo2/v1",
                body,
                Some(session.token()),
                None,
            )
            .await?;
        let envelope = guard(envelope)?;
        serde_json::from_value(envelope.data)
            .map_err(|error| CloudError::Malformed(format!("userinfo: {error}")))
    }

    pub async fn devices(&self, session: &CloudSession) -> CloudResult<Vec<CloudDevice>> {
        let body = json!({ "Authorization": session.token() });
        let envelope = self
            .call("/api/transfer/mdlist/v1", body, Some(session.token()), None)
            .await?;
        parse_devices(guard(envelope)?)
    }

    pub async fn statuses(
        &self,
        session: &CloudSession,
        uuids: &[String],
    ) -> CloudResult<Vec<DeviceStatus>> {
        if uuids.is_empty() {
            return Ok(Vec::new());
        }
        let body =
            serde_json::Value::Array(uuids.iter().map(|uuid| json!({ "uuid": uuid })).collect());
        let envelope = self
            .call(
                "/api/transfer/devstatus/v1",
                body,
                Some(session.token()),
                None,
            )
            .await?;
        parse_statuses(guard(envelope)?)
    }

    pub async fn devices_with_status(
        &self,
        session: &CloudSession,
    ) -> CloudResult<Vec<crate::provider::Device>> {
        let devices = self.devices(session).await?;
        let uuids: Vec<String> = devices.iter().map(|device| device.uuid.clone()).collect();
        let statuses = self.statuses(session, &uuids).await?;
        let online: HashMap<&str, bool> = statuses
            .iter()
            .map(|status| (status.uuid.as_str(), status.online()))
            .collect();
        Ok(devices
            .iter()
            .map(|device| {
                let is_online = online.get(device.uuid.as_str()).copied().unwrap_or(false);
                device.to_provider(is_online)
            })
            .collect())
    }
}

fn guard(envelope: Envelope) -> CloudResult<Envelope> {
    if envelope.is_ok() {
        return Ok(envelope);
    }
    Err(classify(envelope.code, envelope.msg.as_deref()))
}

fn classify(code: i32, vendor: Option<&str>) -> CloudError {
    if code == CODE_TOKEN_EXPIRED {
        return CloudError::Expired;
    }
    if code == CODE_BAD_CREDENTIALS {
        return CloudError::Credentials;
    }
    CloudError::Rejected(user_message(code, vendor))
}

#[derive(Deserialize)]
struct CaptchaData {
    #[serde(default)]
    image: String,
    #[serde(default, rename = "codeToken")]
    code_token: String,
}

fn parse_captcha(envelope: Envelope) -> CloudResult<Captcha> {
    let data: CaptchaData = serde_json::from_value(envelope.data)
        .map_err(|error| CloudError::Malformed(format!("captcha: {error}")))?;
    if data.image.is_empty() || data.code_token.is_empty() {
        return Err(CloudError::Malformed(
            "captcha reply is missing the image or its token".to_string(),
        ));
    }
    Ok(Captcha {
        image: data.image,
        token: data.code_token,
    })
}

fn parse_devices(envelope: Envelope) -> CloudResult<Vec<CloudDevice>> {
    let list = envelope
        .data
        .get("data")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
    serde_json::from_value(list).map_err(|error| CloudError::Malformed(format!("mdlist: {error}")))
}

fn parse_statuses(envelope: Envelope) -> CloudResult<Vec<DeviceStatus>> {
    let list = if envelope.data.is_array() {
        envelope.data
    } else {
        envelope
            .data
            .get("data")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new()))
    };
    serde_json::from_value(list)
        .map_err(|error| CloudError::Malformed(format!("devstatus: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::types::{CODE_BAD_CREDENTIALS, CODE_OK};

    fn envelope(value: serde_json::Value) -> Envelope {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn parses_captcha_reply() {
        let captcha = parse_captcha(envelope(json!({
            "code": CODE_OK,
            "data": { "image": "data:image/jpg;base64,AAA", "codeToken": "tok-1" }
        })))
        .unwrap();
        assert_eq!(captcha.token, "tok-1");
        assert!(captcha.image.starts_with("data:image/jpg;base64,"));
    }

    #[test]
    fn rejects_captcha_without_token() {
        let error = parse_captcha(envelope(json!({
            "code": CODE_OK,
            "data": { "image": "data:image/jpg;base64,AAA" }
        })))
        .unwrap_err();
        assert!(matches!(error, CloudError::Malformed(_)));
    }

    #[test]
    fn parses_device_list_nested_under_data() {
        let devices = parse_devices(envelope(json!({
            "code": CODE_OK,
            "data": { "data": [
                { "uuid": "u1", "nickname": "garden", "username": "owner", "deviceType": "IPC" },
                { "uuid": "u2", "nickname": "gate", "username": "owner" }
            ]}
        })))
        .unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].display_name(), "garden");
        assert!(devices[0].extra.contains_key("deviceType"));
    }

    #[test]
    fn parses_status_list() {
        let statuses = parse_statuses(envelope(json!({
            "code": CODE_OK,
            "data": [
                { "uuid": "u1", "status": "online", "wakeUpStatus": "awake" },
                { "uuid": "u2", "status": "notfound" }
            ]
        })))
        .unwrap();
        assert!(statuses[0].online());
        assert!(!statuses[1].online());
    }

    #[test]
    fn classifies_expired_and_rejected() {
        assert!(matches!(
            classify(CODE_TOKEN_EXPIRED, None),
            CloudError::Expired
        ));
        let error = classify(CODE_BAD_CREDENTIALS, None);
        assert_eq!(error.to_string(), "email or password is wrong");
    }

    #[test]
    fn login_token_is_read_from_the_envelope_root() {
        let parsed = envelope(json!({ "code": CODE_OK, "accessToken": "tok-9" }));
        assert_eq!(parsed.token(), Some("tok-9"));
    }
}
