use std::path::PathBuf;

use super::{CameraProvider, Device, ProviderError, StreamKind, StreamTicket, VendorSession};
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
        let raw = std::fs::read_to_string(path)?;
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

#[async_trait::async_trait]
impl CameraProvider for Fixture {
    async fn login(&self, user: &str, pass: &str) -> Result<VendorSession, ProviderError> {
        let path = self.dir.join("account.json");
        let raw = std::fs::read_to_string(path)?;
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
        let path = self.dir.join(format!("{device}_ch{channel}_{name}.m3u8"));
        if !path.exists() {
            return Err(ProviderError::Upstream(format!(
                "fixture stream missing: {}",
                path.display()
            )));
        }
        Ok(StreamTicket {
            kind: StreamKind::Hls,
            url: format!("/media/{device}/ch{channel}/{name}.m3u8"),
            expires_in: 300,
        })
    }
}
