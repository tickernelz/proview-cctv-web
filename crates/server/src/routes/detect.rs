use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use proview_core::config::Config;
use proview_core::device::{ControlError, SessionError};
use proview_core::dvrip::client::{Connection, DvrError};
use proview_core::dvrip::detect::{AlarmOverview, DetectError, Detector, DetectorPatch, Gate};
use proview_core::dvrip::settings::{SettingsError, WriteReport, WriteWindow};
use proview_core::provider::cloud::CloudProvider;
use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;

use super::control::controls_for;
use super::{fail, Failure};
use crate::config::ModeStore;

pub fn routes() -> Vec<rocket::route::Route> {
    routes![overview, write, stop_gate, stop_alarm]
}

const BACKUP_ENV: &str = "PROVIEW_SETTINGS_BACKUP_DIR";
const WINDOW_ENV: &str = "PROVIEW_SETTINGS_WINDOW_MS";
const DEFAULT_WINDOW: Duration = Duration::from_secs(20);

fn window() -> Duration {
    match std::env::var(WINDOW_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        Some(ms) if ms > 0 => Duration::from_millis(ms),
        _ => DEFAULT_WINDOW,
    }
}

fn backup_dir() -> PathBuf {
    if let Some(dir) = std::env::var(BACKUP_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return PathBuf::from(dir);
    }
    std::env::var("PROVIEW_DATA_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("settings-backups")
}

fn settings_failure(error: SettingsError) -> Failure {
    let status = match &error {
        SettingsError::Invalid { .. } => Status::BadRequest,
        SettingsError::Missing { .. } => Status::NotImplemented,
        SettingsError::Backup { .. } => Status::InternalServerError,
        SettingsError::Slow { .. } => Status::GatewayTimeout,
        SettingsError::WriteFailed { .. } => Status::BadGateway,
        SettingsError::RevertFailed { .. } => Status::Conflict,
        SettingsError::Device(DvrError::Rejected { ret, .. }) => match ret {
            607 => Status::NotImplemented,
            103 | 107 => Status::Forbidden,
            _ => Status::BadGateway,
        },
        SettingsError::Device(_) => Status::BadGateway,
    };
    fail(status, error.to_string())
}

fn detect_failure(error: DetectError) -> Failure {
    match error {
        DetectError::Unsupported { .. } => fail(Status::NotImplemented, error.to_string()),
        DetectError::Settings(error) => settings_failure(error),
    }
}

fn control_failure(error: ControlError) -> Failure {
    let status = match &error {
        ControlError::Unsupported { .. } => Status::NotImplemented,
        ControlError::NotConfigured(_) => Status::BadRequest,
        ControlError::Blocked { .. } => Status::Conflict,
        ControlError::Timeout { .. } => Status::GatewayTimeout,
        ControlError::Device(_) => Status::BadGateway,
    };
    fail(status, error.to_string())
}

async fn session(
    device: Option<&str>,
    cfg: &Config,
    cloud: &Arc<CloudProvider>,
    modes: &ModeStore,
    store: &Store,
    cookies: &CookieJar<'_>,
    window: Duration,
) -> Result<Connection, Failure> {
    let controls = controls_for(device, cfg, cloud, modes, store, cookies)?;
    let control = controls.dvrip().map_err(control_failure)?;
    control
        .session_within(window)
        .await
        .map_err(|error| match error {
            SessionError::Timeout { address, .. } => settings_failure(SettingsError::Slow {
                block: "login",
                detail: format!("connect and login to {address} never answered"),
            }),
            SessionError::Failed(error) => control_failure(error),
        })
}

fn hung(error: &SettingsError) -> bool {
    matches!(
        error,
        SettingsError::Slow { .. } | SettingsError::RevertFailed { .. }
    )
}

fn detect_hung(error: &DetectError) -> bool {
    matches!(error, DetectError::Settings(error) if hung(error))
}

async fn close(connection: &mut Connection, hung: bool, window: Duration) {
    if !hung {
        let _ = connection.logout_within(window).await;
    }
}

#[derive(Serialize)]
pub struct OverviewBody {
    pub ok: bool,
    pub overview: AlarmOverview,
}

#[get("/device/detect?<device>")]
async fn overview(
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<OverviewBody>, Failure> {
    let window = window();
    let mut connection =
        session(device.as_deref(), cfg, cloud, modes, store, cookies, window).await?;
    let read = connection.alarm_overview_within(window).await;
    close(
        &mut connection,
        read.as_ref().err().map(hung).unwrap_or(false),
        window,
    )
    .await;
    let overview = read.map_err(settings_failure)?;
    Ok(Json(OverviewBody { ok: true, overview }))
}

#[derive(Serialize)]
pub struct WriteBody {
    pub ok: bool,
    pub report: WriteReport,
}

#[allow(clippy::too_many_arguments)]
#[post("/device/detect/<detector>?<device>", data = "<body>")]
async fn write(
    detector: &str,
    device: Option<String>,
    body: String,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<WriteBody>, Failure> {
    let target = Detector::parse(detector).ok_or_else(|| {
        let known: Vec<&str> = Detector::ALL.iter().map(|item| item.key()).collect();
        fail(
            Status::NotFound,
            format!(
                "unknown detector {detector}; use one of {}",
                known.join(", ")
            ),
        )
    })?;
    let patch: DetectorPatch = serde_json::from_str(&body)
        .map_err(|error| fail(Status::BadRequest, format!("patch: {error}")))?;
    let window = window();
    let mut connection =
        session(device.as_deref(), cfg, cloud, modes, store, cookies, window).await?;
    let written = connection
        .write_detector(
            target,
            &patch,
            &WriteWindow::with_timeout(backup_dir(), window),
        )
        .await;
    close(
        &mut connection,
        written.as_ref().err().map(detect_hung).unwrap_or(false),
        window,
    )
    .await;
    let report = written.map_err(detect_failure)?;
    Ok(Json(WriteBody { ok: true, report }))
}

#[derive(Serialize)]
pub struct StopGateBody {
    pub ok: bool,
    pub gate: Gate,
}

#[get("/device/detect/alarm/stop?<device>")]
async fn stop_gate(
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StopGateBody>, Failure> {
    let window = window();
    let mut connection =
        session(device.as_deref(), cfg, cloud, modes, store, cookies, window).await?;
    let read = connection.manual_stop_gate_within(window).await;
    close(
        &mut connection,
        read.as_ref().err().map(hung).unwrap_or(false),
        window,
    )
    .await;
    let gate = read.map_err(settings_failure)?;
    Ok(Json(StopGateBody { ok: true, gate }))
}

#[derive(Serialize)]
pub struct StopBody {
    pub ok: bool,
    pub ret: i64,
}

#[post("/device/detect/alarm/stop?<device>")]
async fn stop_alarm(
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StopBody>, Failure> {
    let window = window();
    let mut connection =
        session(device.as_deref(), cfg, cloud, modes, store, cookies, window).await?;
    let stopped = connection.stop_manual_alarm_within(window).await;
    close(
        &mut connection,
        stopped.as_ref().err().map(detect_hung).unwrap_or(false),
        window,
    )
    .await;
    let ret = stopped.map_err(detect_failure)?;
    Ok(Json(StopBody { ok: true, ret }))
}
