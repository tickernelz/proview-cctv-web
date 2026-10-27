use std::path::PathBuf;

use super::{
    Alarm, Device, DeviceSettings, Provider, ProviderError, ProviderSource, Recording, StreamKind,
    StreamTicket, VendorSession,
};
use crate::config::Config;

pub struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    pub fn new(cfg: &Config) -> Self {
        Self {
            dir: PathBuf::from(&cfg.fixture_dir),
        }
    }

    fn load_devices(&self) -> Result<Vec<Device>, ProviderError> {
        let path = self.dir.join("devices.json");
        let raw = std::fs::read_to_string(&path).map_err(|error| {
            ProviderError::Misconfigured(format!(
                "sample data is missing at {} ({error}); point the sample data folder under Settings, Configuration, Advanced at a directory with devices.json, or use the vendor cloud",
                path.display()
            ))
        })?;
        serde_json::from_str(&raw).map_err(|e| ProviderError::Upstream(e.to_string()))
    }
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

const NO_ACCOUNT_CLOUD: &str =
    "the fixture provider has no cloud account, recordings or alarms; it only replays recorded clips";

#[async_trait::async_trait]
impl Provider for Fixture {
    fn source(&self) -> ProviderSource {
        ProviderSource::Fixture
    }

    async fn login(&self, user: &str, pass: &str) -> Result<VendorSession, ProviderError> {
        let path = self.dir.join("account.json");
        let raw = std::fs::read_to_string(&path).map_err(|error| {
            ProviderError::Misconfigured(format!(
                "sample account is missing at {} ({error}); point the sample data folder under Settings, Configuration, Advanced at a directory with account.json, or use the vendor cloud",
                path.display()
            ))
        })?;
        let expected: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| ProviderError::Upstream(e.to_string()))?;
        let want_user = expected
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let want_pass = expected
            .get("password")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if constant_time_eq(user, want_user) && constant_time_eq(pass, want_pass) {
            Ok(VendorSession {
                account: user.to_string(),
                token: None,
            })
        } else {
            Err(ProviderError::BadCredentials)
        }
    }

    async fn devices(&self, _session: &VendorSession) -> Result<Vec<Device>, ProviderError> {
        self.load_devices()
    }

    async fn stream(
        &self,
        _session: &VendorSession,
        device: &str,
        channel: u32,
        high: bool,
    ) -> Result<StreamTicket, ProviderError> {
        let devices = self.load_devices()?;
        let found = devices
            .iter()
            .find(|d| d.id == device)
            .ok_or_else(|| ProviderError::UnknownDevice(device.to_string()))?;
        if channel < 1 || channel > found.channels {
            return Err(ProviderError::UnknownDevice(format!(
                "{device} channel {channel}"
            )));
        }
        let name = if high { "main" } else { "sub" };
        let path = self.dir.join(format!("{device}_ch{channel}_{name}.h264"));
        if !path.exists() {
            return Err(ProviderError::Upstream(format!(
                "no recorded clip at {}",
                path.display()
            )));
        }
        Ok(StreamTicket {
            kind: StreamKind::Ws,
            url: format!("/ws/live?device={device}&channel={channel}&quality={name}"),
            expires_in: 300,
            source: ProviderSource::Fixture,
        })
    }

    async fn playback(
        &self,
        _session: &VendorSession,
        _device: &str,
        _channel: u32,
    ) -> Result<Vec<Recording>, ProviderError> {
        Err(ProviderError::NotImplemented(NO_ACCOUNT_CLOUD))
    }

    async fn alarms(
        &self,
        _session: &VendorSession,
        _limit: u32,
    ) -> Result<Vec<Alarm>, ProviderError> {
        Err(ProviderError::NotImplemented(NO_ACCOUNT_CLOUD))
    }

    async fn settings(
        &self,
        _session: &VendorSession,
        _device: &str,
    ) -> Result<DeviceSettings, ProviderError> {
        Err(ProviderError::NotImplemented(NO_ACCOUNT_CLOUD))
    }

    async fn save_settings(
        &self,
        _session: &VendorSession,
        _settings: &DeviceSettings,
    ) -> Result<(), ProviderError> {
        Err(ProviderError::NotImplemented(NO_ACCOUNT_CLOUD))
    }
}
