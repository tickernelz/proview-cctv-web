use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};

use crate::config::Config;
use crate::dvrip::client::Connection as DvripConnection;
use crate::dvrip::control::{RecordMode, WhiteLightMode};
use crate::onvif::{self, Client as OnvifClient, OnvifError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Onvif,
    Dvrip,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Onvif => "onvif",
            Backend::Dvrip => "dvrip",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lens {
    Top,
    Bottom,
    Device,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Supported,
    Blocked,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LensCapability {
    pub lens: Lens,
    pub feature: String,
    pub state: State,
    pub backend: Backend,
    pub evidence: String,
}

impl LensCapability {
    fn supported(lens: Lens, feature: &str, backend: Backend, evidence: impl Into<String>) -> Self {
        Self {
            lens,
            feature: feature.to_string(),
            state: State::Supported,
            backend,
            evidence: evidence.into(),
        }
    }

    fn blocked(lens: Lens, feature: &str, backend: Backend, evidence: impl Into<String>) -> Self {
        Self {
            lens,
            feature: feature.to_string(),
            state: State::Blocked,
            backend,
            evidence: evidence.into(),
        }
    }
}

#[derive(Debug)]
pub enum ControlError {
    Unsupported {
        feature: &'static str,
        backend: Backend,
    },
    NotConfigured(&'static str),
    Blocked {
        operation: &'static str,
        reason: &'static str,
    },
    Device(String),
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ControlError::Unsupported { feature, backend } => write!(
                f,
                "{feature} is not served by the {} backend",
                backend.as_str()
            ),
            ControlError::NotConfigured(name) => write!(f, "{name} is not configured"),
            ControlError::Blocked { operation, reason } => {
                write!(f, "{operation} is blocked: {reason}")
            }
            ControlError::Device(detail) => write!(f, "device: {detail}"),
        }
    }
}

impl std::error::Error for ControlError {}

impl From<OnvifError> for ControlError {
    fn from(error: OnvifError) -> Self {
        match error {
            OnvifError::Blocked { operation, reason } => {
                ControlError::Blocked { operation, reason }
            }
            other => ControlError::Device(other.to_string()),
        }
    }
}

impl From<crate::dvrip::client::DvrError> for ControlError {
    fn from(error: crate::dvrip::client::DvrError) -> Self {
        ControlError::Device(error.to_string())
    }
}

pub const TOP_LENS_BLOCKER: &str = "the PTZ node is bound to the lower lens; a ContinuousMove shifted the lower lens and left the top lens at (0,0) px (ZHA-3907)";

#[async_trait]
pub trait DeviceControl: Send + Sync {
    fn backend(&self) -> Backend;

    async fn capabilities(&self) -> Result<Vec<LensCapability>, ControlError>;

    fn not_served(&self, feature: &'static str) -> ControlError {
        ControlError::Unsupported {
            feature,
            backend: self.backend(),
        }
    }

    async fn move_ptz(
        &self,
        _lens: Lens,
        _pan: f32,
        _tilt: f32,
        _zoom: f32,
        _hold: Duration,
    ) -> Result<(), ControlError> {
        Err(self.not_served("ptz move"))
    }

    async fn stop_ptz(&self) -> Result<(), ControlError> {
        Err(self.not_served("ptz stop"))
    }

    async fn presets(&self) -> Result<Vec<onvif::Preset>, ControlError> {
        Err(self.not_served("ptz presets"))
    }

    async fn goto_preset(&self, _token: &str) -> Result<(), ControlError> {
        Err(self.not_served("ptz goto preset"))
    }

    async fn set_preset(&self, _name: Option<&str>) -> Result<String, ControlError> {
        Err(self.not_served("ptz set preset"))
    }

    async fn white_light(
        &self,
        _mode: Option<WhiteLightMode>,
    ) -> Result<Option<String>, ControlError> {
        Err(self.not_served("white light"))
    }

    async fn picture_flip(
        &self,
        _flip: Option<bool>,
        _mirror: Option<bool>,
    ) -> Result<(Option<String>, Option<String>), ControlError> {
        Err(self.not_served("picture flip"))
    }

    async fn record_mode(&self, _mode: Option<RecordMode>) -> Result<Option<String>, ControlError> {
        Err(self.not_served("record mode"))
    }

    async fn snapshot(&self) -> Result<Vec<u8>, ControlError> {
        Err(self.not_served("snapshot"))
    }

    async fn settings(&self) -> Result<Value, ControlError> {
        Err(self.not_served("device settings"))
    }
}

pub struct OnvifControl {
    client: OnvifClient,
}

impl OnvifControl {
    pub fn new(cfg: &Config) -> Result<Self, ControlError> {
        let endpoint = cfg
            .device_onvif
            .clone()
            .ok_or(ControlError::NotConfigured("PROVIEW_DEVICE_ONVIF"))?;
        let endpoint = if endpoint.starts_with("http") {
            endpoint
        } else {
            format!("http://{endpoint}/onvif/device_service")
        };
        Ok(Self {
            client: OnvifClient::new(endpoint),
        })
    }

    pub fn client(&self) -> &OnvifClient {
        &self.client
    }
}

#[async_trait]
impl DeviceControl for OnvifControl {
    fn backend(&self) -> Backend {
        Backend::Onvif
    }

    async fn capabilities(&self) -> Result<Vec<LensCapability>, ControlError> {
        let information = self.client.device_information().await?;
        let node = self.client.ptz_node().await;
        let presets = self.client.presets().await.unwrap_or_default();
        let mut rows = Vec::new();
        match node {
            Ok(node) => {
                let evidence = format!(
                    "GetNodes: node {}, {} presets, home {}, aux {:?}; device {} firmware {}",
                    node.token,
                    node.max_presets,
                    node.home_supported,
                    node.auxiliary_commands,
                    information.model,
                    information.firmware
                );
                rows.push(LensCapability::supported(
                    Lens::Bottom,
                    "ptz move",
                    Backend::Onvif,
                    format!("ContinuousMove with an explicit Timeout; {evidence}"),
                ));
                rows.push(LensCapability::supported(
                    Lens::Bottom,
                    "ptz preset",
                    Backend::Onvif,
                    format!("GetPresets returned {} tokens ({evidence})", presets.len()),
                ));
                rows.push(LensCapability::blocked(
                    Lens::Bottom,
                    "ptz patrol",
                    Backend::Onvif,
                    "ONVIF ver20 PTZ on this firmware exposes no preset-tour operation; GetNodes lists only wiper auxiliary commands",
                ));
            }
            Err(error) => {
                rows.push(LensCapability::blocked(
                    Lens::Bottom,
                    "ptz move",
                    Backend::Onvif,
                    format!("GetNodes failed: {error}"),
                ));
            }
        }
        rows.push(LensCapability::blocked(
            Lens::Top,
            "ptz move",
            Backend::Onvif,
            TOP_LENS_BLOCKER,
        ));
        rows.push(LensCapability::blocked(
            Lens::Device,
            "ptz relative move",
            Backend::Onvif,
            "refused in code before any packet leaves: the call takes every device service down for 35-45 s, measured on this firmware",
        ));
        rows.push(LensCapability::blocked(
            Lens::Device,
            "ptz position read-back",
            Backend::Onvif,
            "GetStatus answers ter:ActionNotSupported; the logical position is tracked by the application and restored with GotoPreset",
        ));
        match self.client.imaging_settings("V_SRC_000").await {
            Ok(settings) => rows.push(LensCapability::supported(
                Lens::Device,
                "imaging settings",
                Backend::Onvif,
                format!("GetImagingSettings: {settings}"),
            )),
            Err(error) => rows.push(LensCapability::blocked(
                Lens::Device,
                "imaging settings",
                Backend::Onvif,
                format!("GetImagingSettings failed: {error}"),
            )),
        }
        rows.push(LensCapability::supported(
            Lens::Device,
            "device information",
            Backend::Onvif,
            format!(
                "GetDeviceInformation: {} {} serial {}",
                information.manufacturer, information.model, information.serial
            ),
        ));
        Ok(rows)
    }

    async fn move_ptz(
        &self,
        lens: Lens,
        pan: f32,
        tilt: f32,
        zoom: f32,
        hold: Duration,
    ) -> Result<(), ControlError> {
        if lens != Lens::Bottom {
            return Err(ControlError::Blocked {
                operation: "ptz move on the top lens",
                reason: TOP_LENS_BLOCKER,
            });
        }
        let hold = hold.clamp(Duration::from_millis(100), Duration::from_secs(5));
        self.client.continuous_move(pan, tilt, zoom, hold).await?;
        Ok(())
    }

    async fn stop_ptz(&self) -> Result<(), ControlError> {
        self.client.stop().await?;
        Ok(())
    }

    async fn presets(&self) -> Result<Vec<onvif::Preset>, ControlError> {
        Ok(self.client.presets().await?)
    }

    async fn goto_preset(&self, token: &str) -> Result<(), ControlError> {
        self.client.goto_preset(token).await?;
        Ok(())
    }

    async fn set_preset(&self, name: Option<&str>) -> Result<String, ControlError> {
        Ok(self.client.set_preset(name).await?)
    }

    async fn settings(&self) -> Result<Value, ControlError> {
        let information = self.client.device_information().await?;
        let profiles = self.client.profiles().await?;
        let profile = profiles.first().ok_or(ControlError::Device(
            "device reports no media profile".to_string(),
        ))?;
        let encoder = match profile.encoder.as_deref() {
            Some(token) => self.client.encoder_configuration(token).await.ok(),
            None => None,
        };
        let imaging = match profile.video_source.as_deref() {
            Some(token) => self.client.imaging_settings(token).await.ok(),
            None => None,
        };
        let interfaces = self.client.network_interfaces().await.unwrap_or_default();
        Ok(json!({
            "device": information,
            "profile": profile,
            "encoder": encoder,
            "imaging": imaging,
            "interfaces": interfaces,
        }))
    }
}

pub struct DvripControl {
    address: String,
    user: String,
    password: String,
}

impl DvripControl {
    pub fn new(cfg: &Config) -> Result<Self, ControlError> {
        let address = cfg
            .device_host
            .clone()
            .ok_or(ControlError::NotConfigured("PROVIEW_DEVICE_HOST"))?;
        Ok(Self {
            address,
            user: cfg.device_user.clone(),
            password: cfg.device_pass.clone(),
        })
    }

    pub async fn session(&self) -> Result<DvripConnection, ControlError> {
        let mut connection = DvripConnection::connect(&self.address)
            .await
            .map_err(|error| ControlError::Device(format!("connect {}: {error}", self.address)))?;
        connection.login(&self.user, &self.password).await?;
        Ok(connection)
    }
}

#[async_trait]
impl DeviceControl for DvripControl {
    fn backend(&self) -> Backend {
        Backend::Dvrip
    }

    async fn capabilities(&self) -> Result<Vec<LensCapability>, ControlError> {
        let mut connection = self.session().await?;
        let reported = connection.capabilities().await?;
        let flip = connection.picture_flags(0).await;
        let _ = connection.logout().await;
        Ok(vec![
            LensCapability::supported(
                Lens::Device,
                "white light",
                Backend::Dvrip,
                reported.white_light.evidence,
            ),
            flip_capability(flip, reported.picture_flip),
            LensCapability::supported(
                Lens::Device,
                "snapshot",
                Backend::Dvrip,
                reported.snapshot.evidence,
            ),
            LensCapability::supported(
                Lens::Device,
                "record mode",
                Backend::Dvrip,
                reported.record_mode.evidence,
            ),
            LensCapability::blocked(
                Lens::Device,
                "two-way audio",
                Backend::Dvrip,
                format!(
                    "{} but the server has no capture path to feed the talk channel",
                    reported.talk.evidence
                ),
            ),
            LensCapability::blocked(
                Lens::Device,
                "ptz patrol over dvrip",
                Backend::Dvrip,
                format!(
                    "{} ; the shipped PTZ transport is ONVIF, this backend is the fallback",
                    reported.ptz_tour.evidence
                ),
            ),
        ])
    }

    async fn move_ptz(
        &self,
        lens: Lens,
        pan: f32,
        tilt: f32,
        _zoom: f32,
        hold: Duration,
    ) -> Result<(), ControlError> {
        if lens != Lens::Bottom {
            return Err(ControlError::Blocked {
                operation: "ptz move on the top lens",
                reason: TOP_LENS_BLOCKER,
            });
        }
        let mut connection = self.session().await?;
        let result = connection.ptz_step(pan, tilt, hold).await;
        let _ = connection.logout().await;
        result.map_err(ControlError::from)
    }

    async fn white_light(
        &self,
        mode: Option<WhiteLightMode>,
    ) -> Result<Option<String>, ControlError> {
        let mut connection = self.session().await?;
        let result = match mode {
            Some(mode) => connection
                .set_white_light(mode)
                .await
                .map(|_| Some(mode.wire().to_string())),
            None => connection.white_light_mode().await,
        };
        let _ = connection.logout().await;
        result.map_err(ControlError::from)
    }

    async fn picture_flip(
        &self,
        flip: Option<bool>,
        mirror: Option<bool>,
    ) -> Result<(Option<String>, Option<String>), ControlError> {
        let mut connection = self.session().await?;
        let mut result = Ok((None, None));
        if flip.is_some() || mirror.is_some() {
            result = connection
                .set_picture_flip(0, flip, mirror)
                .await
                .map(|_| (None, None));
        }
        if result.is_ok() {
            result = connection.picture_flags(0).await;
        }
        let _ = connection.logout().await;
        result.map_err(ControlError::from)
    }

    async fn record_mode(&self, mode: Option<RecordMode>) -> Result<Option<String>, ControlError> {
        let mut connection = self.session().await?;
        let result = match mode {
            Some(mode) => connection
                .set_record_mode(mode)
                .await
                .map(|_| Some(mode.wire().to_string())),
            None => connection.record_mode().await,
        };
        let _ = connection.logout().await;
        result.map_err(ControlError::from)
    }

    async fn snapshot(&self) -> Result<Vec<u8>, ControlError> {
        let mut connection = self.session().await?;
        let result = connection.snapshot(0).await;
        let _ = connection.logout().await;
        result.map_err(ControlError::from)
    }
}

fn flip_capability(
    live_read: Result<(Option<String>, Option<String>), crate::dvrip::client::DvrError>,
    reported: crate::dvrip::control::Capability,
) -> LensCapability {
    match live_read {
        Ok((flip, mirror)) => LensCapability::supported(
            Lens::Device,
            "picture flip",
            Backend::Dvrip,
            format!(
                "Camera.Param[0] read live: PictureFlip={}, PictureMirror={}",
                flip.unwrap_or_else(|| "?".to_string()),
                mirror.unwrap_or_else(|| "?".to_string())
            ),
        ),
        Err(error) => LensCapability::blocked(
            Lens::Device,
            "picture flip",
            Backend::Dvrip,
            format!(
                "{error}; earlier read during this session reported {}",
                reported.evidence
            ),
        ),
    }
}

pub struct Controls {
    pub onvif: Option<OnvifControl>,
    pub dvrip: Option<DvripControl>,
    onvif_error: Option<String>,
    dvrip_error: Option<String>,
}

impl Controls {
    pub fn from_config(cfg: &Config) -> Self {
        let (onvif, onvif_error) = match OnvifControl::new(cfg) {
            Ok(control) => (Some(control), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let (dvrip, dvrip_error) = match DvripControl::new(cfg) {
            Ok(control) => (Some(control), None),
            Err(error) => (None, Some(error.to_string())),
        };
        Self {
            onvif,
            dvrip,
            onvif_error,
            dvrip_error,
        }
    }

    pub fn onvif(&self) -> Result<&OnvifControl, ControlError> {
        self.onvif.as_ref().ok_or_else(|| {
            ControlError::Device(
                self.onvif_error
                    .clone()
                    .unwrap_or_else(|| "onvif backend unavailable".to_string()),
            )
        })
    }

    pub fn dvrip(&self) -> Result<&DvripControl, ControlError> {
        self.dvrip.as_ref().ok_or_else(|| {
            ControlError::Device(
                self.dvrip_error
                    .clone()
                    .unwrap_or_else(|| "dvrip backend unavailable".to_string()),
            )
        })
    }

    pub async fn capability_report(&self) -> Vec<LensCapability> {
        let mut rows = Vec::new();
        match &self.onvif {
            Some(control) => match control.capabilities().await {
                Ok(reported) => rows.extend(reported),
                Err(error) => rows.push(LensCapability::blocked(
                    Lens::Device,
                    "onvif transport",
                    Backend::Onvif,
                    error.to_string(),
                )),
            },
            None => rows.push(LensCapability::blocked(
                Lens::Device,
                "onvif transport",
                Backend::Onvif,
                self.onvif_error.clone().unwrap_or_default(),
            )),
        }
        match &self.dvrip {
            Some(control) => match control.capabilities().await {
                Ok(reported) => rows.extend(reported),
                Err(error) => rows.push(LensCapability::blocked(
                    Lens::Device,
                    "dvrip settings",
                    Backend::Dvrip,
                    error.to_string(),
                )),
            },
            None => rows.push(LensCapability::blocked(
                Lens::Device,
                "dvrip settings",
                Backend::Dvrip,
                self.dvrip_error.clone().unwrap_or_default(),
            )),
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn top_lens_ptz_is_refused_before_any_request_is_sent() {
        let config = test_config(Some("http://127.0.0.1:1/onvif/device_service"), None);
        let control = OnvifControl::new(&config).unwrap();
        match control
            .move_ptz(Lens::Top, 0.5, 0.0, 0.0, Duration::from_millis(200))
            .await
        {
            Err(ControlError::Blocked { operation, reason }) => {
                assert!(operation.contains("top lens"));
                assert!(reason.contains("lower lens"));
            }
            other => panic!("expected the top lens to be refused, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_backend_without_its_configuration_fails_before_the_network() {
        let config = test_config(None, None);
        assert!(matches!(
            OnvifControl::new(&config),
            Err(ControlError::NotConfigured("PROVIEW_DEVICE_ONVIF"))
        ));
        assert!(matches!(
            DvripControl::new(&config),
            Err(ControlError::NotConfigured("PROVIEW_DEVICE_HOST"))
        ));
    }

    #[tokio::test]
    async fn features_the_onvif_backend_does_not_serve_name_the_backend() {
        let config = test_config(Some("http://127.0.0.1:1/onvif/device_service"), None);
        let control = OnvifControl::new(&config).unwrap();
        match control.white_light(None).await {
            Err(ControlError::Unsupported { feature, backend }) => {
                assert_eq!(feature, "white light");
                assert_eq!(backend, Backend::Onvif);
            }
            other => panic!("expected unsupported, got {other:?}"),
        }
    }

    #[test]
    fn control_errors_map_to_http_status_expectations() {
        let blocked = ControlError::Blocked {
            operation: "relative move",
            reason: "takes the device down",
        };
        assert!(blocked.to_string().contains("blocked"));
        assert!(ControlError::Unsupported {
            feature: "white light",
            backend: Backend::Onvif
        }
        .to_string()
        .contains("onvif"));
    }

    fn test_config(onvif: Option<&str>, host: Option<&str>) -> crate::config::Config {
        crate::config::Config {
            provider: crate::config::ProviderKind::Fixture,
            cloud_base: "https://pub-token.xmeye.net".to_string(),
            device_user: "admin".to_string(),
            device_pass: "device-password".to_string(),
            fixture_dir: "fixtures".to_string(),
            device_host: host.map(str::to_string),
            device_rtsp: None,
            device_onvif: onvif.map(str::to_string),
            session_ttl_secs: 3600,
        }
    }
}
