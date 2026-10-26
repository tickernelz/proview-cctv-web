pub mod fixture;
pub mod proview;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::{Config, ProviderKind};

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
    #[error("device id unknown: {0}")]
    UnknownDevice(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamKind {
    Hls,
    Rtsp,
    Flv,
}

#[derive(Debug, Clone)]
pub struct VendorSession {
    pub account: String,
}

#[async_trait::async_trait]
pub trait CameraProvider: Send + Sync {
    async fn login(&self, user: &str, pass: &str) -> Result<VendorSession, ProviderError>;
    async fn devices(&self, session: &VendorSession) -> Result<Vec<Device>, ProviderError>;
    async fn stream(
        &self,
        session: &VendorSession,
        device: &str,
        channel: u32,
        high: bool,
    ) -> Result<StreamTicket, ProviderError>;
}

pub fn build(cfg: &Config) -> Box<dyn CameraProvider> {
    match cfg.provider {
        ProviderKind::Fixture => Box::new(fixture::Fixture::new(cfg)),
        ProviderKind::Proview => Box::new(proview::Proview::new(cfg)),
    }
}
