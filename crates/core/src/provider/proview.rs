use super::{CameraProvider, Device, ProviderError, StreamKind, StreamTicket, VendorSession};
use crate::config::Config;

const LOGIN_PATH: &str = "/webservice/app/user/logInByAccount";

pub struct Proview {
    base: String,
    device_user: String,
    device_pass: String,
    http: reqwest::Client,
}

impl Proview {
    pub fn new(cfg: &Config) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("proview-cctv-web/0.1")
            .cookie_store(true)
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("http client");
        Self {
            base: cfg.cloud_base.clone(),
            device_user: cfg.device_user.clone(),
            device_pass: cfg.device_pass.clone(),
            http,
        }
    }
}

#[async_trait::async_trait]
impl CameraProvider for Proview {
    async fn login(&self, user: &str, pass: &str) -> Result<VendorSession, ProviderError> {
        let url = format!("{}{}", self.base, LOGIN_PATH);
        let response = self
            .http
            .post(&url)
            .form(&[
                ("username", user),
                ("password", pass),
                ("time", &now_millis().to_string()),
                ("token", ""),
            ])
            .send()
            .await
            .map_err(|e| ProviderError::Unreachable(e.to_string()))?;

        let status = response.status();
        let cookie_present = response
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .any(|v| v.to_str().map(|s| !s.is_empty()).unwrap_or(false));

        if !status.is_success() {
            return Err(ProviderError::Upstream(format!("login HTTP {status}")));
        }
        if !cookie_present {
            return Err(ProviderError::NotImplemented(
                "account login envelope (CssCenter JSON) is not solved yet; vendor returned no session cookie",
            ));
        }
        Ok(VendorSession {
            account: user.to_string(),
        })
    }

    async fn devices(&self, _session: &VendorSession) -> Result<Vec<Device>, ProviderError> {
        Err(ProviderError::NotImplemented(
            "device list call (XmCloud/GetDevsCfgInfo) is mapped but its envelope is unverified",
        ))
    }

    async fn stream(
        &self,
        _session: &VendorSession,
        _device: &str,
        _channel: u32,
        _high: bool,
    ) -> Result<StreamTicket, ProviderError> {
        let _ = (&self.device_user, &self.device_pass, StreamKind::Rtsp);
        Err(ProviderError::NotImplemented(
            "live stream needs the device session: LAN RTSP or CSS HLS, neither wired yet",
        ))
    }
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
