pub mod cloud;
pub mod fixture;
pub mod local;
pub mod router;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::{Config, ProviderKind};

pub use router::{Notice, Resolution, Router};

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("invalid credentials")]
    BadCredentials,
    #[error("vendor endpoint not reachable: {0}")]
    Unreachable(String),
    #[error("vendor protocol not implemented yet: {0}")]
    NotImplemented(&'static str),
    #[error("upstream error: {0}")]
    Upstream(String),
    #[error("{0}")]
    Misconfigured(String),
    #[error("device id unknown: {0}")]
    UnknownDevice(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ProviderError {
    pub fn is_transport(&self) -> bool {
        matches!(self, ProviderError::Unreachable(_) | ProviderError::Io(_))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Cloud,
    Local,
}

impl Mode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "cloud" => Some(Mode::Cloud),
            "local" => Some(Mode::Local),
            _ => None,
        }
    }

    pub fn from_env() -> Self {
        std::env::var("PROVIEW_MODE")
            .ok()
            .as_deref()
            .and_then(Mode::parse)
            .unwrap_or_default()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Cloud => "cloud",
            Mode::Local => "local",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderSource {
    Cloud,
    Local,
    Fixture,
}

impl ProviderSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderSource::Cloud => "cloud",
            ProviderSource::Local => "local",
            ProviderSource::Fixture => "fixture",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub model: String,
    pub online: bool,
    pub channels: u32,
    pub lan_host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamTicket {
    pub kind: StreamKind,
    pub url: String,
    pub expires_in: u64,
    pub source: ProviderSource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamKind {
    Hls,
    Rtsp,
    Flv,
    Mp4,
    Ws,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    pub id: String,
    pub device: String,
    pub channel: u32,
    pub start: String,
    pub duration_secs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alarm {
    pub id: String,
    pub device: String,
    pub channel: u32,
    pub kind: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceSettings {
    pub device: String,
    pub channel: u32,
    pub stream: String,
    pub resolution: String,
    pub fps: u32,
    pub audio: bool,
}

#[derive(Debug, Clone)]
pub struct VendorSession {
    pub account: String,
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn source(&self) -> ProviderSource;

    async fn login(&self, user: &str, pass: &str) -> Result<VendorSession, ProviderError>;

    async fn devices(&self, session: &VendorSession) -> Result<Vec<Device>, ProviderError>;

    async fn stream(
        &self,
        session: &VendorSession,
        device: &str,
        channel: u32,
        high: bool,
    ) -> Result<StreamTicket, ProviderError>;

    async fn playback(
        &self,
        session: &VendorSession,
        device: &str,
        channel: u32,
    ) -> Result<Vec<Recording>, ProviderError>;

    async fn alarms(
        &self,
        session: &VendorSession,
        limit: u32,
    ) -> Result<Vec<Alarm>, ProviderError>;

    async fn settings(
        &self,
        session: &VendorSession,
        device: &str,
    ) -> Result<DeviceSettings, ProviderError>;

    async fn save_settings(
        &self,
        session: &VendorSession,
        settings: &DeviceSettings,
    ) -> Result<(), ProviderError>;
}

pub fn configured_source(cfg: &Config) -> ProviderSource {
    match cfg.provider {
        ProviderKind::Fixture => ProviderSource::Fixture,
        ProviderKind::Proview => ProviderSource::Cloud,
    }
}

pub fn build(cfg: &Config) -> Box<dyn Provider> {
    match cfg.provider {
        ProviderKind::Fixture => Box::new(fixture::Fixture::new(cfg)),
        ProviderKind::Proview => Box::new(cloud::CloudProvider::new(cfg)),
    }
}

pub fn plan(cfg: &Config, mode: Mode) -> Resolution {
    match mode {
        Mode::Cloud => Resolution {
            mode,
            source: configured_source(cfg),
            notice: None,
        },
        Mode::Local => match local::LocalProvider::from_config(cfg) {
            Some(_) => Resolution {
                mode,
                source: ProviderSource::Local,
                notice: None,
            },
            None => {
                let source = configured_source(cfg);
                Resolution {
                    mode,
                    source,
                    notice: Some(Notice {
                        from: Mode::Local,
                        to: source,
                        message: local::MISSING_HOST.to_string(),
                    }),
                }
            }
        },
    }
}
