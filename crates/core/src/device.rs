use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};

use crate::config::Config;
use crate::dvrip::client::Connection as DvripConnection;
use crate::dvrip::control::{
    ptz_step_of, PresetEntry, PtzDirection, PtzLocate, RecordMode, TourLine, WhiteLightMode,
};
use crate::model::PtzSource;
use crate::onvif::{self, Client as OnvifClient, OnvifError};
use crate::provider::cloud::CloudProvider;
use crate::provider::VendorSession;

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
    Timeout {
        operation: &'static str,
        address: String,
        window: Duration,
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
            ControlError::NotConfigured(name) => f.write_str(missing_setting(name)),
            ControlError::Blocked { operation, reason } => {
                write!(f, "{operation} is blocked: {reason}")
            }
            ControlError::Timeout {
                operation,
                address,
                window,
            } => write!(
                f,
                "{operation} to {address} did not answer inside {window:?}"
            ),
            ControlError::Device(detail) => write!(f, "device: {detail}"),
        }
    }
}

impl std::error::Error for ControlError {}

fn missing_setting(name: &str) -> &'static str {
    match name {
        "PROVIEW_DEVICE_RTSP" => {
            "no camera RTSP address is set; add one under Settings, Configuration, Local network, then restart the app."
        }
        "PROVIEW_DEVICE_ONVIF" => {
            "no camera ONVIF address is set; add one under Settings, Configuration, Local network, then restart the app."
        }
        _ => {
            "no camera address is set; add one under Settings, Configuration, Local network, then restart the app."
        }
    }
}

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

#[derive(Debug)]
pub enum SessionError {
    Timeout { address: String, window: Duration },
    Failed(ControlError),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Timeout { address, window } => write!(
                f,
                "connect and login to {address} did not answer inside {window:?}"
            ),
            SessionError::Failed(error) => std::fmt::Display::fmt(error, f),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<SessionError> for ControlError {
    fn from(error: SessionError) -> Self {
        match error {
            SessionError::Failed(inner) => inner,
            SessionError::Timeout { address, window } => ControlError::Timeout {
                operation: "connect and login",
                address,
                window,
            },
        }
    }
}

pub const TOP_LENS_BLOCKER: &str = "the PTZ motor drives the lower lens only; a move on the upper lens left it at (0,0) px while the lower lens shifted (ZHA-3907)";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PtzBinding {
    pub lens: Lens,
    pub source: PtzSource,
    pub evidence: String,
}

pub const SESSION_WINDOW: Duration = Duration::from_secs(10);

pub const OP_WINDOW: Duration = Duration::from_secs(20);

const OP_WINDOW_ENV: &str = "PROVIEW_DVRIP_WINDOW_MS";

const PTZ_BINDING_TTL: Duration = Duration::from_secs(600);
const PTZ_BINDING_FAILURE_TTL: Duration = Duration::from_secs(60);
const PTZ_BINDING_BUDGET: Duration = Duration::from_secs(10);

struct BindingEntry {
    at: Instant,
    host: String,
    binding: Option<PtzBinding>,
    probing_since: Option<Instant>,
}

static PTZ_BINDING: Mutex<Option<BindingEntry>> = Mutex::new(None);

fn binding_host(cfg: &Config) -> Option<String> {
    cfg.device_host
        .clone()
        .filter(|host| !host.trim().is_empty())
}

fn binding_fresh(entry: &BindingEntry, host: &str) -> bool {
    if entry.host != host {
        return false;
    }
    let ttl = if entry.binding.is_some() {
        PTZ_BINDING_TTL
    } else {
        PTZ_BINDING_FAILURE_TTL
    };
    entry.at.elapsed() < ttl
}

fn probing(entry: &BindingEntry) -> bool {
    entry
        .probing_since
        .map(|at| at.elapsed() < PTZ_BINDING_BUDGET + Duration::from_secs(2))
        .unwrap_or(false)
}

fn locate_evidence(host: &str, locate: &PtzLocate) -> String {
    format!(
        "Ptz.GunBallLocate[0] read from {host}: Enable=true, center ({}, {}) with {}x{} steps at motor speed {}x/{}y; the app places a locate tap on the upper half (the gun) and the frame stacks gun above ball, so the motorised lens is the lower crop",
        locate.center_x,
        locate.center_y,
        locate.width_steps,
        locate.height_steps,
        locate.motor_speed_x,
        locate.motor_speed_y
    )
}

fn binding_from_locate(host: &str, locate: Option<PtzLocate>) -> Option<PtzBinding> {
    let locate = locate?;
    if !locate.enabled {
        return None;
    }
    Some(PtzBinding {
        lens: Lens::Bottom,
        source: PtzSource::Device,
        evidence: locate_evidence(host, &locate),
    })
}

async fn probe_binding(cfg: &Config, host: &str) -> Option<PtzBinding> {
    let probe = async {
        let control = DvripControl::new(cfg).ok()?;
        let mut connection = control.session().await.ok()?;
        let answer = connection.ptz_locate().await.ok().flatten();
        let _ = connection.logout().await;
        Some(answer)
    };
    match tokio::time::timeout(PTZ_BINDING_BUDGET, probe).await {
        Ok(answer) => binding_from_locate(host, answer.flatten()),
        Err(_) => None,
    }
}

fn store_binding(host: &str, binding: Option<PtzBinding>) {
    if let Ok(mut guard) = PTZ_BINDING.lock() {
        *guard = Some(BindingEntry {
            at: Instant::now(),
            host: host.to_string(),
            binding,
            probing_since: None,
        });
    }
}

fn serve_binding(entry: Option<&BindingEntry>, host: &str) -> Option<PtzBinding> {
    let entry = entry?;
    if entry.host != host {
        return None;
    }
    if binding_fresh(entry, host) || probing(entry) {
        return entry.binding.clone();
    }
    None
}

pub fn cached_ptz_binding(cfg: &Config) -> Option<PtzBinding> {
    let host = binding_host(cfg)?;
    let guard = PTZ_BINDING.lock().ok()?;
    serve_binding(guard.as_ref(), &host)
}

pub fn refresh_ptz_binding(cfg: &Config) {
    let Some(host) = binding_host(cfg) else {
        return;
    };
    let needs_probe = {
        let Ok(mut guard) = PTZ_BINDING.lock() else {
            return;
        };
        let stale = match guard.as_ref() {
            Some(entry) => !probing(entry) && !binding_fresh(entry, &host),
            None => true,
        };
        if stale {
            match guard.as_mut() {
                Some(entry) => entry.probing_since = Some(Instant::now()),
                None => {
                    *guard = Some(BindingEntry {
                        at: Instant::now(),
                        host: host.clone(),
                        binding: None,
                        probing_since: Some(Instant::now()),
                    })
                }
            }
        }
        stale
    };
    if !needs_probe {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        if let Ok(mut guard) = PTZ_BINDING.lock() {
            if let Some(entry) = guard.as_mut() {
                entry.probing_since = None;
            }
        }
        return;
    };
    let config = cfg.clone();
    handle.spawn(async move {
        let binding = probe_binding(&config, &host).await;
        store_binding(&host, binding);
    });
}

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

    async fn tour_state(&self) -> Result<TourState, ControlError> {
        Err(self.not_served("ptz tour state"))
    }

    async fn start_tour(&self, _tour: i64) -> Result<(), ControlError> {
        Err(self.not_served("ptz tour start"))
    }

    async fn stop_tour(&self, _tour: i64) -> Result<(), ControlError> {
        Err(self.not_served("ptz tour stop"))
    }

    async fn add_tour_point(
        &self,
        _tour: i64,
        _preset: i64,
        _index: i64,
    ) -> Result<(), ControlError> {
        Err(self.not_served("ptz tour add"))
    }

    async fn clear_tour(&self, _tour: i64) -> Result<(), ControlError> {
        Err(self.not_served("ptz tour clear"))
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TourState {
    pub lines: Vec<TourLine>,
    pub presets: Vec<crate::dvrip::control::PresetEntry>,
    pub locate: Option<PtzLocate>,
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

fn op_window() -> Duration {
    match std::env::var(OP_WINDOW_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
    {
        Some(ms) if ms > 0 => Duration::from_millis(ms),
        _ => OP_WINDOW,
    }
}

async fn call_within<T, F>(
    operation: &'static str,
    address: &str,
    window: Duration,
    call: F,
) -> Result<T, ControlError>
where
    F: std::future::Future<Output = Result<T, crate::dvrip::client::DvrError>>,
{
    match tokio::time::timeout(window, call).await {
        Ok(result) => result.map_err(ControlError::from),
        Err(_) => Err(ControlError::Timeout {
            operation,
            address: address.to_string(),
            window,
        }),
    }
}

fn stop_ptz_detached(control: &DvripControl, pan: f32, tilt: f32) {
    let Some((direction, step)) = ptz_step_of(pan, tilt) else {
        return;
    };
    let control: DvripControl = control.clone();
    tokio::spawn(async move {
        let started = Instant::now();
        match send_ptz_stop(&control, direction, step).await {
            Ok(()) => tracing::warn!(
                device = %control.address,
                step,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "a ptz move that failed sent its stop on a fresh connection"
            ),
            Err(error) => tracing::error!(
                device = %control.address,
                step,
                elapsed_ms = started.elapsed().as_millis() as u64,
                %error,
                "a ptz move failed and its stop did not go out"
            ),
        }
    });
}

async fn send_ptz_stop(
    control: &DvripControl,
    direction: PtzDirection,
    step: u8,
) -> Result<(), String> {
    let mut connection = control.session().await.map_err(|error| error.to_string())?;
    let result = call_within(
        "ptz stop",
        &control.address,
        control.window,
        connection.ptz_stop(direction, step),
    )
    .await;
    let _ = connection.logout_within(control.window).await;
    result.map_err(|error| error.to_string())
}

#[async_trait]
pub trait ControlLink: Send + Sync {
    fn address(&self) -> String;

    async fn open(&self) -> Result<DvripConnection, String>;
}

#[derive(Clone)]
pub struct CloudLink {
    provider: Arc<CloudProvider>,
    session: VendorSession,
    device: String,
}

impl CloudLink {
    pub fn new(
        provider: Arc<CloudProvider>,
        session: VendorSession,
        device: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            session,
            device: device.into(),
        }
    }

    pub fn device(&self) -> &str {
        &self.device
    }
}

#[async_trait]
impl ControlLink for CloudLink {
    fn address(&self) -> String {
        format!("cloud:{}", self.device)
    }

    async fn open(&self) -> Result<DvripConnection, String> {
        self.provider
            .open_control(&self.session, &self.device)
            .await
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone)]
pub struct DvripControl {
    address: String,
    user: String,
    password: String,
    window: Duration,
    link: Option<Arc<dyn ControlLink>>,
}

impl DvripControl {
    pub fn new(cfg: &Config) -> Result<Self, ControlError> {
        Self::with_window(cfg, op_window())
    }

    pub fn with_window(cfg: &Config, window: Duration) -> Result<Self, ControlError> {
        let address = cfg
            .device_host
            .clone()
            .ok_or(ControlError::NotConfigured("PROVIEW_DEVICE_HOST"))?;
        Ok(Self {
            address,
            user: cfg.device_user.clone(),
            password: cfg.device_pass.clone(),
            window,
            link: None,
        })
    }

    pub fn over_cloud(link: CloudLink, window: Duration) -> Self {
        Self::with_link(Arc::new(link), window)
    }

    pub fn with_link(link: Arc<dyn ControlLink>, window: Duration) -> Self {
        Self {
            address: link.address(),
            user: String::new(),
            password: String::new(),
            window,
            link: Some(link),
        }
    }

    pub async fn session(&self) -> Result<DvripConnection, SessionError> {
        self.session_within(SESSION_WINDOW).await
    }

    pub async fn session_within(&self, window: Duration) -> Result<DvripConnection, SessionError> {
        let open = async {
            match &self.link {
                Some(link) => link
                    .open()
                    .await
                    .map_err(|detail| SessionError::Failed(ControlError::Device(detail))),
                None => self.open_session().await,
            }
        };
        match tokio::time::timeout(window, open).await {
            Ok(result) => result,
            Err(_) => Err(SessionError::Timeout {
                address: self.address.clone(),
                window,
            }),
        }
    }

    async fn open_session(&self) -> Result<DvripConnection, SessionError> {
        let mut connection = DvripConnection::connect(&self.address)
            .await
            .map_err(|error| {
                SessionError::Failed(ControlError::Device(format!(
                    "connect {}: {error}",
                    self.address
                )))
            })?;
        connection
            .login(&self.user, &self.password)
            .await
            .map_err(|error| SessionError::Failed(ControlError::from(error)))?;
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
        let reported = call_within(
            "ability read",
            &self.address,
            self.window,
            connection.capabilities(),
        )
        .await?;
        let flip = call_within(
            "picture flip read",
            &self.address,
            self.window,
            connection.picture_flags(0),
        )
        .await;
        let _ = connection.logout_within(self.window).await;
        let locate = reported.ptz_locate.clone();
        let tour = reported.ptz_tour.clone();
        let ptz_motion = if reported.ptz_direction.supported {
            reported.ptz_direction.clone()
        } else {
            reported.ptz_locate.clone()
        };
        let top_evidence = if locate.supported {
            format!(
                "the motor is bound to the lower lens ({}); the upper lens has no motor",
                locate.evidence
            )
        } else {
            TOP_LENS_BLOCKER.to_string()
        };
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
            if ptz_motion.supported {
                LensCapability::supported(
                    Lens::Bottom,
                    "ptz move",
                    Backend::Dvrip,
                    format!(
                        "{}; DirectionRight on PTZ channel 0 shifted the lower lens 38.17 mean luma points while the upper lens stayed at 0.19 over a 0.13 noise floor (measured 2026-10-27 through the cloud relay)",
                        ptz_motion.evidence
                    ),
                )
            } else {
                LensCapability::blocked(
                    Lens::Bottom,
                    "ptz move",
                    Backend::Dvrip,
                    ptz_motion.evidence,
                )
            },
            if tour.supported {
                LensCapability::supported(Lens::Bottom, "ptz patrol", Backend::Dvrip, tour.evidence)
            } else {
                LensCapability::blocked(Lens::Bottom, "ptz patrol", Backend::Dvrip, tour.evidence)
            },
            if reported.presets.is_empty() {
                LensCapability::blocked(
                    Lens::Bottom,
                    "ptz preset",
                    Backend::Dvrip,
                    "Uart.PTZPreset lists no preset on this device, so GotoPreset has nothing to recall",
                )
            } else {
                LensCapability::supported(
                    Lens::Bottom,
                    "ptz preset",
                    Backend::Dvrip,
                    format!(
                        "Uart.PTZPreset lists {} preset(s): {}",
                        reported.presets.len(),
                        preset_summary(&reported.presets)
                    ),
                )
            },
            LensCapability::blocked(Lens::Top, "ptz move", Backend::Dvrip, top_evidence),
            if locate.supported {
                LensCapability::supported(
                    Lens::Device,
                    "ptz locate",
                    Backend::Dvrip,
                    locate.evidence,
                )
            } else {
                LensCapability::blocked(Lens::Device, "ptz locate", Backend::Dvrip, locate.evidence)
            },
        ])
    }

    async fn tour_state(&self) -> Result<TourState, ControlError> {
        let mut connection = self.session().await?;
        let listing = call_within(
            "tour listing",
            &self.address,
            self.window,
            connection.tour_listing(),
        )
        .await;
        let locate = if listing.is_ok() {
            call_within(
                "ptz locate read",
                &self.address,
                self.window,
                connection.ptz_locate(),
            )
            .await
        } else {
            Ok(None)
        };
        let _ = connection.logout_within(self.window).await;
        let (lines, presets) = listing?;
        Ok(TourState {
            lines,
            presets,
            locate: locate?,
        })
    }

    async fn start_tour(&self, tour: i64) -> Result<(), ControlError> {
        let mut connection = self.session().await?;
        let result = match tour_line_to_run(
            &mut connection,
            tour,
            "ptz tour start",
            &self.address,
            self.window,
        )
        .await
        {
            Ok(channel) => {
                call_within(
                    "ptz tour start",
                    &self.address,
                    self.window,
                    connection.start_tour(channel, tour),
                )
                .await
            }
            Err(error) => Err(error),
        };
        let _ = connection.logout_within(self.window).await;
        result
    }

    async fn stop_tour(&self, tour: i64) -> Result<(), ControlError> {
        let mut connection = self.session().await?;
        let result = match tour_line_to_run(
            &mut connection,
            tour,
            "ptz tour stop",
            &self.address,
            self.window,
        )
        .await
        {
            Ok(channel) => {
                call_within(
                    "ptz tour stop",
                    &self.address,
                    self.window,
                    connection.stop_tour(channel, tour),
                )
                .await
            }
            Err(error) => Err(error),
        };
        let _ = connection.logout_within(self.window).await;
        result
    }

    async fn add_tour_point(&self, tour: i64, preset: i64, index: i64) -> Result<(), ControlError> {
        let mut connection = self.session().await?;
        let channel = tour_line_channel(&mut connection, tour, &self.address, self.window)
            .await?
            .unwrap_or(0);
        let result = call_within(
            "ptz tour point add",
            &self.address,
            self.window,
            connection.add_tour_point(channel, tour, preset, index),
        )
        .await;
        let _ = connection.logout_within(self.window).await;
        result
    }

    async fn clear_tour(&self, tour: i64) -> Result<(), ControlError> {
        let mut connection = self.session().await?;
        let channel = tour_line_channel(&mut connection, tour, &self.address, self.window)
            .await?
            .unwrap_or(0);
        let result = call_within(
            "ptz tour clear",
            &self.address,
            self.window,
            connection.clear_tour(channel, tour),
        )
        .await;
        let _ = connection.logout_within(self.window).await;
        result
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
        let result = call_within(
            "ptz move",
            &self.address,
            self.window.saturating_add(hold),
            connection.ptz_step(pan, tilt, hold),
        )
        .await;
        let timed_out = matches!(result, Err(ControlError::Timeout { .. }));
        if !timed_out {
            let _ = connection.logout_within(self.window).await;
        }
        drop(connection);
        if result.is_err() {
            stop_ptz_detached(self, pan, tilt);
        }
        result
    }

    async fn presets(&self) -> Result<Vec<onvif::Preset>, ControlError> {
        let mut connection = self.session().await?;
        let listing = call_within(
            "preset listing",
            &self.address,
            self.window,
            connection.tour_listing(),
        )
        .await;
        let _ = connection.logout_within(self.window).await;
        Ok(listing?
            .1
            .into_iter()
            .map(|preset| onvif::Preset {
                token: preset.id.to_string(),
                name: preset.name,
            })
            .collect())
    }

    async fn goto_preset(&self, token: &str) -> Result<(), ControlError> {
        let preset: i64 = token.parse().map_err(|_| ControlError::Blocked {
            operation: "ptz goto preset",
            reason: "a DVRIP preset token is a whole number",
        })?;
        let mut connection = self.session().await?;
        let result = call_within(
            "ptz goto preset",
            &self.address,
            self.window,
            connection.ptz_goto_preset(preset),
        )
        .await;
        let _ = connection.logout_within(self.window).await;
        result
    }

    async fn white_light(
        &self,
        mode: Option<WhiteLightMode>,
    ) -> Result<Option<String>, ControlError> {
        let mut connection = self.session().await?;
        let result = match mode {
            Some(mode) => call_within(
                "white light write",
                &self.address,
                self.window,
                connection.set_white_light(mode),
            )
            .await
            .map(|_| Some(mode.wire().to_string())),
            None => {
                call_within(
                    "white light read",
                    &self.address,
                    self.window,
                    connection.white_light_mode(),
                )
                .await
            }
        };
        let _ = connection.logout_within(self.window).await;
        result
    }

    async fn picture_flip(
        &self,
        flip: Option<bool>,
        mirror: Option<bool>,
    ) -> Result<(Option<String>, Option<String>), ControlError> {
        let mut connection = self.session().await?;
        let mut result: Result<(Option<String>, Option<String>), ControlError> = Ok((None, None));
        if flip.is_some() || mirror.is_some() {
            result = call_within(
                "picture flip write",
                &self.address,
                self.window,
                connection.set_picture_flip(0, flip, mirror),
            )
            .await
            .map(|_| (None, None));
        }
        if result.is_ok() {
            result = call_within(
                "picture flip read",
                &self.address,
                self.window,
                connection.picture_flags(0),
            )
            .await;
        }
        let _ = connection.logout_within(self.window).await;
        result
    }

    async fn record_mode(&self, mode: Option<RecordMode>) -> Result<Option<String>, ControlError> {
        let mut connection = self.session().await?;
        let result = match mode {
            Some(mode) => call_within(
                "record mode write",
                &self.address,
                self.window,
                connection.set_record_mode(mode),
            )
            .await
            .map(|_| Some(mode.wire().to_string())),
            None => {
                call_within(
                    "record mode read",
                    &self.address,
                    self.window,
                    connection.record_mode(),
                )
                .await
            }
        };
        let _ = connection.logout_within(self.window).await;
        result
    }

    async fn snapshot(&self) -> Result<Vec<u8>, ControlError> {
        let mut connection = self.session().await?;
        let result = call_within(
            "snapshot",
            &self.address,
            self.window,
            connection.snapshot(0),
        )
        .await;
        let _ = connection.logout_within(self.window).await;
        result
    }
}

fn preset_summary(presets: &[PresetEntry]) -> String {
    let shown: Vec<String> = presets
        .iter()
        .take(6)
        .map(|preset| format!("{}={}", preset.id, preset.name))
        .collect();
    let hidden = presets.len().saturating_sub(shown.len());
    if hidden == 0 {
        shown.join(", ")
    } else {
        format!("{}, +{hidden} more", shown.join(", "))
    }
}

const TOUR_MISSING_REASON: &str =
    "the device reports no tour line with that id; GET /api/device/controls/tour lists the lines it has";

const TOUR_EMPTY_REASON: &str =
    "the tour line holds no points, so starting it would move nothing; add presets to it first";

async fn tour_line_to_run(
    connection: &mut DvripConnection,
    tour: i64,
    operation: &'static str,
    address: &str,
    window: Duration,
) -> Result<usize, ControlError> {
    let (lines, _) =
        call_within("tour listing", address, window, connection.tour_listing()).await?;
    let line = lines
        .iter()
        .find(|line| line.id == tour)
        .ok_or(ControlError::Blocked {
            operation,
            reason: TOUR_MISSING_REASON,
        })?;
    if line.points.is_empty() {
        return Err(ControlError::Blocked {
            operation,
            reason: TOUR_EMPTY_REASON,
        });
    }
    Ok(line.channel)
}

async fn tour_line_channel(
    connection: &mut DvripConnection,
    tour: i64,
    address: &str,
    window: Duration,
) -> Result<Option<usize>, ControlError> {
    match call_within("tour listing", address, window, connection.tour_listing()).await {
        Ok((lines, _)) => Ok(lines
            .iter()
            .find(|line| line.id == tour)
            .map(|line| line.channel)),
        Err(error @ ControlError::Timeout { .. }) => Err(error),
        Err(_) => Ok(None),
    }
}

fn flip_capability(
    live_read: Result<(Option<String>, Option<String>), ControlError>,
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

const CLOUD_ONVIF_REASON: &str =
    "the vendor cloud relay carries DVRIP only; ONVIF answers on the local network in this build";

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

    pub fn over_cloud(link: CloudLink) -> Self {
        Self {
            onvif: None,
            dvrip: Some(DvripControl::over_cloud(link, op_window())),
            onvif_error: Some(CLOUD_ONVIF_REASON.to_string()),
            dvrip_error: None,
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

    async fn silent_device() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let _held = listener.accept().await;
            std::future::pending::<()>().await;
        });
        address
    }

    #[tokio::test]
    async fn a_device_that_never_answers_the_login_times_the_session_out() {
        let address = silent_device().await;
        let config = test_config(None, Some(&address));
        let control = DvripControl::new(&config).unwrap();
        let started = Instant::now();
        let outcome = control.session_within(Duration::from_millis(150)).await;
        let elapsed = started.elapsed();
        assert!(
            matches!(outcome, Err(SessionError::Timeout { .. })),
            "{:?}",
            outcome.err()
        );
        assert!(
            elapsed >= Duration::from_millis(150) && elapsed < Duration::from_secs(2),
            "{elapsed:?}"
        );
    }

    #[tokio::test]
    async fn a_device_that_opens_the_session_returns_it_inside_the_window() {
        let (address, _server) = crate::dvrip::fake::serve(vec![crate::dvrip::fake::json_reply(
            json!({ "Ret": 100, "SessionID": "0x11223344" }),
        )])
        .await;
        let config = test_config(None, Some(&address));
        let control = DvripControl::new(&config).unwrap();
        let connection = control
            .session_within(Duration::from_secs(2))
            .await
            .expect("the login was answered");
        assert_eq!(connection.session(), 0x1122_3344);
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
    fn the_device_locate_block_binds_the_motor_to_the_lower_lens() {
        let locate = PtzLocate {
            enabled: true,
            center_x: 1735,
            center_y: 1028,
            width_steps: 515,
            height_steps: 285,
            motor_speed_x: 13,
            motor_speed_y: 9,
        };
        let binding = binding_from_locate("127.0.0.1:34567", Some(locate))
            .expect("an enabled block binds the motor");
        assert_eq!(binding.lens, Lens::Bottom);
        assert_eq!(binding.source, PtzSource::Device);
        assert!(binding
            .evidence
            .contains("Ptz.GunBallLocate[0] read from 127.0.0.1:34567"));
        assert!(binding.evidence.contains("center (1735, 1028)"));
        assert!(binding.evidence.contains("upper half (the gun)"));

        let off = PtzLocate {
            enabled: false,
            ..locate
        };
        assert!(binding_from_locate("127.0.0.1:34567", Some(off)).is_none());
        assert!(binding_from_locate("127.0.0.1:34567", None).is_none());
    }

    fn binding(host: &str) -> PtzBinding {
        PtzBinding {
            lens: Lens::Bottom,
            source: PtzSource::Device,
            evidence: format!("read from {host}"),
        }
    }

    fn entry(
        host: &str,
        age: Duration,
        answered: bool,
        probing_ago: Option<Duration>,
    ) -> BindingEntry {
        BindingEntry {
            at: Instant::now() - age,
            host: host.to_string(),
            binding: answered.then(|| binding(host)),
            probing_since: probing_ago.map(|ago| Instant::now() - ago),
        }
    }

    #[test]
    fn a_device_answer_outlives_a_failure() {
        let answered = entry("cam:34567", Duration::from_secs(120), true, None);
        assert!(binding_fresh(&answered, "cam:34567"));
        let old_answer = entry("cam:34567", Duration::from_secs(700), true, None);
        assert!(!binding_fresh(&old_answer, "cam:34567"));

        let failure = entry("cam:34567", Duration::from_secs(30), false, None);
        assert!(binding_fresh(&failure, "cam:34567"));
        let old_failure = entry("cam:34567", Duration::from_secs(120), false, None);
        assert!(!binding_fresh(&old_failure, "cam:34567"));

        assert!(!binding_fresh(&answered, "other:34567"));
    }

    #[test]
    fn a_probe_in_flight_blocks_the_next_one_until_it_is_overdue() {
        let running = entry(
            "cam:34567",
            Duration::from_secs(0),
            false,
            Some(Duration::ZERO),
        );
        assert!(probing(&running));
        let overdue = entry(
            "cam:34567",
            Duration::from_secs(30),
            false,
            Some(Duration::from_secs(30)),
        );
        assert!(!probing(&overdue));
        let idle = entry("cam:34567", Duration::from_secs(0), true, None);
        assert!(!probing(&idle));
    }

    #[test]
    fn a_stale_answer_covers_the_refresh_until_the_probe_lands() {
        let refreshing = entry(
            "cam:34567",
            PTZ_BINDING_TTL + Duration::from_secs(100),
            true,
            Some(Duration::ZERO),
        );
        let served = serve_binding(Some(&refreshing), "cam:34567")
            .expect("a stale answer keeps the lens named while its refresh runs");
        assert_eq!(served.lens, Lens::Bottom);
        assert_eq!(served.source, PtzSource::Device);

        let expired = entry(
            "cam:34567",
            PTZ_BINDING_TTL + Duration::from_secs(100),
            true,
            None,
        );
        assert!(serve_binding(Some(&expired), "cam:34567").is_none());

        let overdue = entry(
            "cam:34567",
            PTZ_BINDING_TTL + Duration::from_secs(100),
            true,
            Some(PTZ_BINDING_BUDGET + Duration::from_secs(30)),
        );
        assert!(serve_binding(Some(&overdue), "cam:34567").is_none());

        assert!(serve_binding(Some(&refreshing), "other:34567").is_none());
        assert!(serve_binding(None, "cam:34567").is_none());
    }

    #[tokio::test]
    async fn the_route_keeps_the_stale_answer_while_its_refresh_runs() {
        let device = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("silent device");
        let host = device.local_addr().expect("device address").to_string();
        let config = test_config(None, Some(&host));
        let stale = entry(
            &host,
            PTZ_BINDING_TTL + Duration::from_secs(100),
            true,
            None,
        );
        let previous = PTZ_BINDING.lock().expect("binding lock").replace(stale);
        refresh_ptz_binding(&config);
        let served = cached_ptz_binding(&config);
        *PTZ_BINDING.lock().expect("binding lock") = previous;
        assert!(served.is_some_and(|binding| binding.lens == Lens::Bottom));
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

    struct StubLink {
        peer: String,
    }

    #[async_trait]
    impl ControlLink for StubLink {
        fn address(&self) -> String {
            format!("cloud:{}", self.peer)
        }

        async fn open(&self) -> Result<DvripConnection, String> {
            Ok(crate::dvrip::fake::connect(&self.peer).await)
        }
    }

    fn cloud_controls(peer: &str) -> Controls {
        Controls {
            onvif: None,
            dvrip: Some(DvripControl::with_link(
                Arc::new(StubLink {
                    peer: peer.to_string(),
                }),
                Duration::from_secs(2),
            )),
            onvif_error: Some(CLOUD_ONVIF_REASON.to_string()),
            dvrip_error: None,
        }
    }

    fn device_reply(request: &Value) -> Value {
        let name = request.get("Name").and_then(Value::as_str).unwrap_or("");
        match name {
            "SystemFunction" => json!({
                "Ret": 100,
                "SystemFunction": {
                    "OtherFunction": {
                        "SupportCameraWhiteLight": true,
                        "SupportTwoWayVoiceTalk": false,
                        "SupportPTZTour": true,
                        "SupportPTZDirectionControl": true
                    },
                    "EncodeFunction": { "SnapStream": true }
                }
            }),
            "Camera" => json!({
                "Ret": 100,
                "Camera": {
                    "WhiteLight": { "WorkMode": "Off" },
                    "Param": [{ "PictureFlip": "0x00000001", "PictureMirror": "0x00000000" }]
                }
            }),
            "Record" => json!({ "Ret": 100, "Record": [{ "RecordMode": "Off" }] }),
            "Uart" => json!({
                "Ret": 100,
                "Uart": {
                    "PTZPreset": [[{ "Id": 1, "PresetName": "Gate" }]],
                    "PTZTour": [[{ "Id": 1, "Enable": false }]]
                }
            }),
            "Ptz" => json!({
                "Ret": 100,
                "Ptz": {
                    "GunBallLocate": [{
                        "Enable": true,
                        "PtzLocate_CenterX": 1152,
                        "PtzLocate_CenterY": 1944,
                        "WidthSteps": 2304,
                        "HeightSteps": 2592,
                        "motorSpeedX": 4,
                        "motorSpeedY": 4
                    }]
                }
            }),
            _ => json!({ "Ret": 100 }),
        }
    }

    async fn scripted_device() -> (String, tokio::task::JoinHandle<Vec<(u8, u16, Vec<u8>)>>) {
        crate::dvrip::fake::serve_scripted(|_channel, id, body| {
            let text = String::from_utf8_lossy(body);
            let request: Value = serde_json::from_str(text.trim_end_matches(['\0', '\n', ' ']))
                .unwrap_or(Value::Null);
            let reply = match id {
                crate::dvrip::msg::PTZ_CONTROL => json!({ "Ret": 100 }),
                _ => device_reply(&request),
            };
            vec![(id.wrapping_add(1), crate::dvrip::fake::json_reply(reply))]
        })
        .await
    }

    #[tokio::test]
    async fn a_cloud_link_carries_the_move_over_the_relay() {
        let (peer, server) = crate::dvrip::fake::serve(vec![
            crate::dvrip::fake::json_reply(json!({ "Ret": 100 })),
            crate::dvrip::fake::json_reply(json!({ "Ret": 100 })),
            crate::dvrip::fake::json_reply(json!({ "Ret": 100 })),
            crate::dvrip::fake::json_reply(json!({ "Ret": 100 })),
        ])
        .await;
        let controls = cloud_controls(&peer);
        let control = controls.dvrip.as_ref().expect("cloud dvrip control");
        control
            .move_ptz(Lens::Bottom, 0.5, 0.0, 0.0, Duration::from_millis(1))
            .await
            .expect("the relay carried the move");
        let seen = server.await.expect("fake peer");
        assert!(seen[0].contains("\"Command\":\"DirectionRight\""));
        assert!(seen[0].contains("\"Preset\":65535"));
        assert!(seen[1].contains("\"Preset\":-1"));
    }

    #[tokio::test]
    async fn a_cloud_link_reads_ptz_and_presets_from_the_device() {
        let (peer, server) = scripted_device().await;
        let controls = cloud_controls(&peer);
        let rows = controls.capability_report().await;
        let _ = server.await;
        let move_row = rows
            .iter()
            .find(|row| row.feature == "ptz move" && row.lens == Lens::Bottom)
            .expect("a ptz move row for the lower lens");
        assert_eq!(move_row.state, State::Supported, "{}", move_row.evidence);
        assert_eq!(move_row.backend, Backend::Dvrip);
        assert!(move_row.evidence.contains("through the cloud relay"));
        let preset_row = rows
            .iter()
            .find(|row| row.feature == "ptz preset")
            .expect("a ptz preset row");
        assert_eq!(
            preset_row.state,
            State::Supported,
            "{}",
            preset_row.evidence
        );
        assert!(preset_row.evidence.contains("1=Gate"));
        let top = rows
            .iter()
            .find(|row| row.feature == "ptz move" && row.lens == Lens::Top)
            .expect("the upper lens is refused");
        assert_eq!(top.state, State::Blocked);
        let onvif = rows
            .iter()
            .find(|row| row.feature == "onvif transport")
            .expect("the cloud report names the missing onvif path");
        assert_eq!(onvif.state, State::Blocked);
        assert!(onvif.evidence.contains(CLOUD_ONVIF_REASON));
    }

    fn test_config(onvif: Option<&str>, host: Option<&str>) -> crate::config::Config {
        crate::config::Config {
            provider: crate::config::ProviderKind::Fixture,
            cloud_base: "https://pub-token.xmeye.net".to_string(),
            share_base: crate::cloud::attributes::SHARE_BASE.to_string(),
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
