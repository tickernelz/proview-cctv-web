use super::{
    Alarm, Device, DeviceSettings, Provider, ProviderError, ProviderSource, Recording, StreamKind,
    StreamTicket, VendorSession,
};
use crate::config::Config;
use crate::rtsp::{self, RtspError};
use crate::source::RtspTarget;

pub const MISSING_HOST: &str = "local mode is selected but no camera host is configured (set PROVIEW_DEVICE_RTSP or PROVIEW_DEVICE_HOST); using the cloud provider";

pub struct LocalProvider {
    target: RtspTarget,
    ttl_secs: u64,
}

impl LocalProvider {
    pub fn from_config(cfg: &Config) -> Option<Self> {
        let raw = cfg
            .device_rtsp
            .as_deref()
            .or(cfg.device_host.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let (host, port) = split_host_port(raw, 554);
        Some(Self {
            target: RtspTarget::from_host_port(&host, port)
                .with_credentials(&cfg.device_user, &cfg.device_pass),
            ttl_secs: cfg.session_ttl_secs,
        })
    }

    pub fn with_target(target: RtspTarget, ttl_secs: u64) -> Self {
        Self { target, ttl_secs }
    }

    pub fn target(&self) -> &RtspTarget {
        &self.target
    }
}

fn device_id_is_safe(device: &str) -> bool {
    !device.is_empty()
        && device
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !device.contains("..")
}

pub fn split_host_port(raw: &str, default_port: u16) -> (String, u16) {
    match raw.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => match port.parse::<u16>() {
            Ok(port) => (host.to_string(), port),
            Err(_) => (raw.to_string(), default_port),
        },
        _ => (raw.to_string(), default_port),
    }
}

#[async_trait::async_trait]
impl Provider for LocalProvider {
    fn source(&self) -> ProviderSource {
        ProviderSource::Local
    }

    async fn login(&self, user: &str, pass: &str) -> Result<VendorSession, ProviderError> {
        let target = &self.target;
        match rtsp::describe(&target.host, target.port, "/", user, pass).await {
            Ok(_) => Ok(VendorSession {
                account: "local".to_string(),
            }),
            Err(RtspError::Unauthorized) => Err(ProviderError::BadCredentials),
            Err(RtspError::Io(error)) => Err(ProviderError::Unreachable(format!(
                "{}:{} {error}",
                target.host, target.port
            ))),
            Err(error) => Err(ProviderError::Upstream(error.to_string())),
        }
    }

    async fn devices(&self, _session: &VendorSession) -> Result<Vec<Device>, ProviderError> {
        Ok(vec![Device {
            id: "local".to_string(),
            name: format!("Camera at {}", self.target.host),
            model: "LAN camera".to_string(),
            online: true,
            channels: 1,
            lan_host: Some(format!("{}:{}", self.target.host, self.target.port)),
        }])
    }

    async fn stream(
        &self,
        _session: &VendorSession,
        device: &str,
        channel: u32,
        high: bool,
    ) -> Result<StreamTicket, ProviderError> {
        if !device_id_is_safe(device) {
            return Err(ProviderError::UnknownDevice(device.to_string()));
        }
        if channel == 0 || channel > 1 {
            return Err(ProviderError::UnknownDevice(format!(
                "{device} channel {channel}"
            )));
        }
        let quality = if high { "main" } else { "sub" };
        Ok(StreamTicket {
            kind: StreamKind::Ws,
            url: format!("/ws/live?device={device}&channel={channel}&quality={quality}"),
            expires_in: self.ttl_secs,
            source: ProviderSource::Local,
        })
    }

    async fn playback(
        &self,
        _session: &VendorSession,
        _device: &str,
        _channel: u32,
    ) -> Result<Vec<Recording>, ProviderError> {
        Err(ProviderError::NotImplemented(
            "LAN playback needs the DVRIP file-search call; the reference IPC answers login with ret 203",
        ))
    }

    async fn alarms(
        &self,
        _session: &VendorSession,
        _limit: u32,
    ) -> Result<Vec<Alarm>, ProviderError> {
        Err(ProviderError::NotImplemented(
            "LAN alarms need the DVRIP event subscription, which is not solved yet",
        ))
    }

    async fn settings(
        &self,
        _session: &VendorSession,
        _device: &str,
    ) -> Result<DeviceSettings, ProviderError> {
        Err(ProviderError::NotImplemented(
            "LAN device settings need DVRIP ConfigGet, which is not solved yet",
        ))
    }

    async fn save_settings(
        &self,
        _session: &VendorSession,
        _settings: &DeviceSettings,
    ) -> Result<(), ProviderError> {
        Err(ProviderError::NotImplemented(
            "LAN device settings need DVRIP ConfigSet, which is not solved yet",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::split_host_port;

    #[test]
    fn host_port_splitting_keeps_a_bare_host_on_the_default_port() {
        assert_eq!(
            split_host_port("192.168.0.100", 554),
            ("192.168.0.100".to_string(), 554)
        );
        assert_eq!(
            split_host_port("192.168.0.100:8554", 554),
            ("192.168.0.100".to_string(), 8554)
        );
        assert_eq!(
            split_host_port("camera.local", 554),
            ("camera.local".to_string(), 554)
        );
    }
}
