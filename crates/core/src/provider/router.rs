use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use super::local::LocalProvider;
use super::{
    build, Alarm, Device, DeviceSettings, Mode, Provider, ProviderError, ProviderSource, Recording,
    StreamTicket, VendorSession,
};
use crate::config::Config;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub from: Mode,
    pub to: ProviderSource,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub mode: Mode,
    pub source: ProviderSource,
    pub notice: Option<Notice>,
}

pub struct Router {
    primary: Box<dyn Provider>,
    fallback: Option<Box<dyn Provider>>,
    resolution: RwLock<Resolution>,
}

impl Router {
    pub fn new(cfg: &Config, mode: Mode) -> Self {
        let cloud = build(cfg);
        match mode {
            Mode::Cloud => Self {
                resolution: RwLock::new(super::plan(cfg, mode)),
                primary: cloud,
                fallback: None,
            },
            Mode::Local => match LocalProvider::from_config(cfg) {
                Some(local) => Self {
                    resolution: RwLock::new(super::plan(cfg, mode)),
                    primary: Box::new(local),
                    fallback: Some(cloud),
                },
                None => Self {
                    resolution: RwLock::new(super::plan(cfg, mode)),
                    primary: cloud,
                    fallback: None,
                },
            },
        }
    }

    pub fn with(
        primary: Box<dyn Provider>,
        fallback: Option<Box<dyn Provider>>,
        mode: Mode,
    ) -> Self {
        let source = primary.source();
        Self {
            primary,
            fallback,
            resolution: RwLock::new(Resolution {
                mode,
                source,
                notice: None,
            }),
        }
    }

    pub fn resolution(&self) -> Resolution {
        self.resolution
            .read()
            .expect("resolution lock poisoned")
            .clone()
    }

    fn active(&self) -> &dyn Provider {
        let resolution = self.resolution.read().expect("resolution lock poisoned");
        match &self.fallback {
            Some(fallback) if resolution.source == fallback.source() => fallback.as_ref(),
            _ => self.primary.as_ref(),
        }
    }

    fn take_over(&self, err: &ProviderError) -> Option<&dyn Provider> {
        if !err.is_transport() {
            return None;
        }
        let fallback = self.fallback.as_deref()?;
        let mut resolution = self.resolution.write().expect("resolution lock poisoned");
        if resolution.source != fallback.source() {
            resolution.source = fallback.source();
            resolution.notice = Some(Notice {
                from: resolution.mode,
                to: fallback.source(),
                message: err.to_string(),
            });
        }
        Some(fallback)
    }

    async fn retry_devices(
        &self,
        err: ProviderError,
        session: &VendorSession,
    ) -> Result<Vec<Device>, ProviderError> {
        match self.take_over(&err) {
            Some(fallback) => fallback.devices(session).await,
            None => Err(err),
        }
    }

    async fn retry_login(
        &self,
        err: ProviderError,
        user: &str,
        pass: &str,
    ) -> Result<VendorSession, ProviderError> {
        match self.take_over(&err) {
            Some(fallback) => fallback.login(user, pass).await,
            None => Err(err),
        }
    }

    async fn retry_stream(
        &self,
        err: ProviderError,
        session: &VendorSession,
        device: &str,
        channel: u32,
        high: bool,
    ) -> Result<StreamTicket, ProviderError> {
        match self.take_over(&err) {
            Some(fallback) => fallback.stream(session, device, channel, high).await,
            None => Err(err),
        }
    }

    async fn retry_playback(
        &self,
        err: ProviderError,
        session: &VendorSession,
        device: &str,
        channel: u32,
    ) -> Result<Vec<Recording>, ProviderError> {
        match self.take_over(&err) {
            Some(fallback) => fallback.playback(session, device, channel).await,
            None => Err(err),
        }
    }

    async fn retry_alarms(
        &self,
        err: ProviderError,
        session: &VendorSession,
        limit: u32,
    ) -> Result<Vec<Alarm>, ProviderError> {
        match self.take_over(&err) {
            Some(fallback) => fallback.alarms(session, limit).await,
            None => Err(err),
        }
    }

    async fn retry_settings(
        &self,
        err: ProviderError,
        session: &VendorSession,
        device: &str,
    ) -> Result<DeviceSettings, ProviderError> {
        match self.take_over(&err) {
            Some(fallback) => fallback.settings(session, device).await,
            None => Err(err),
        }
    }

    async fn retry_save_settings(
        &self,
        err: ProviderError,
        session: &VendorSession,
        settings: &DeviceSettings,
    ) -> Result<(), ProviderError> {
        match self.take_over(&err) {
            Some(fallback) => fallback.save_settings(session, settings).await,
            None => Err(err),
        }
    }
}

#[async_trait::async_trait]
impl Provider for Router {
    fn source(&self) -> ProviderSource {
        self.resolution().source
    }

    async fn login(&self, user: &str, pass: &str) -> Result<VendorSession, ProviderError> {
        match self.active().login(user, pass).await {
            Ok(session) => Ok(session),
            Err(err) => self.retry_login(err, user, pass).await,
        }
    }

    async fn devices(&self, session: &VendorSession) -> Result<Vec<Device>, ProviderError> {
        match self.active().devices(session).await {
            Ok(devices) => Ok(devices),
            Err(err) => self.retry_devices(err, session).await,
        }
    }

    async fn stream(
        &self,
        session: &VendorSession,
        device: &str,
        channel: u32,
        high: bool,
    ) -> Result<StreamTicket, ProviderError> {
        match self.active().stream(session, device, channel, high).await {
            Ok(ticket) => Ok(ticket),
            Err(err) => self.retry_stream(err, session, device, channel, high).await,
        }
    }

    async fn playback(
        &self,
        session: &VendorSession,
        device: &str,
        channel: u32,
    ) -> Result<Vec<Recording>, ProviderError> {
        match self.active().playback(session, device, channel).await {
            Ok(clips) => Ok(clips),
            Err(err) => self.retry_playback(err, session, device, channel).await,
        }
    }

    async fn alarms(
        &self,
        session: &VendorSession,
        limit: u32,
    ) -> Result<Vec<Alarm>, ProviderError> {
        match self.active().alarms(session, limit).await {
            Ok(alarms) => Ok(alarms),
            Err(err) => self.retry_alarms(err, session, limit).await,
        }
    }

    async fn settings(
        &self,
        session: &VendorSession,
        device: &str,
    ) -> Result<DeviceSettings, ProviderError> {
        match self.active().settings(session, device).await {
            Ok(settings) => Ok(settings),
            Err(err) => self.retry_settings(err, session, device).await,
        }
    }

    async fn save_settings(
        &self,
        session: &VendorSession,
        settings: &DeviceSettings,
    ) -> Result<(), ProviderError> {
        match self.active().save_settings(session, settings).await {
            Ok(()) => Ok(()),
            Err(err) => self.retry_save_settings(err, session, settings).await,
        }
    }
}
