use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use proview_core::config::Config;
use proview_core::device::{ControlError, Controls, SessionError};
use proview_core::dvrip::client::Connection;
use proview_core::dvrip::control::RecordMode;
use proview_core::dvrip::encode_osd::{
    EncodeCapabilityView, OsdPatch, OsdState, SmartEncodeState, SmartMode,
};
use proview_core::dvrip::settings::{
    EncodePatch, EncodeState, NetworkPatch, OverwriteMode, SettingsError, Stream, WriteReport,
    WriteWindow,
};
use proview_core::dvrip::storage_card::{check_packet_length, FormatStep, RecordState};
use proview_core::provider::cloud::CloudProvider;
use proview_core::session::Store;
use rocket::form::FromForm;
use rocket::http::{CookieJar, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::serde::json::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::control::controls_for;
use super::{fail, Failure};
use crate::config::ModeStore;

pub fn routes() -> Vec<rocket::route::Route> {
    routes![
        encode_read,
        encode_write,
        network_read,
        network_write,
        storage_read,
        storage_write,
        storage_format,
        record_read,
        record_write,
        osd_read,
        osd_write,
        osd_dot,
    ]
}

const BACKUP_ENV: &str = "PROVIEW_SETTINGS_BACKUP_DIR";
const WINDOW_ENV: &str = "PROVIEW_SETTINGS_WINDOW_MS";
const FORMAT_WINDOW_ENV: &str = "PROVIEW_FORMAT_WINDOW_MS";
const DEFAULT_WINDOW: Duration = Duration::from_secs(20);
const DEFAULT_FORMAT_WINDOW: Duration = Duration::from_secs(60);
pub const FORMAT_CONFIRMATION: &str = "FORMAT";

fn backup_dir() -> PathBuf {
    let explicit = std::env::var(BACKUP_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty());
    if let Some(dir) = explicit {
        return PathBuf::from(dir);
    }
    let data = std::env::var("PROVIEW_DATA_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    data.join("settings-backups")
}

fn window_from(env: &str, fallback: Duration) -> Duration {
    match std::env::var(env)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        Some(ms) if ms > 0 => Duration::from_millis(ms),
        _ => fallback,
    }
}

fn settings_window() -> Duration {
    window_from(WINDOW_ENV, DEFAULT_WINDOW)
}

fn format_window() -> Duration {
    window_from(FORMAT_WINDOW_ENV, DEFAULT_FORMAT_WINDOW)
}

fn write_window() -> WriteWindow {
    WriteWindow::with_timeout(backup_dir(), settings_window())
}

fn settings_failure(error: SettingsError) -> Failure {
    let status = match &error {
        SettingsError::Invalid { .. } => Status::BadRequest,
        SettingsError::Missing { .. } => Status::NotImplemented,
        SettingsError::Backup { .. } => Status::InternalServerError,
        SettingsError::Slow { .. } => Status::GatewayTimeout,
        SettingsError::WriteFailed { .. } => Status::BadGateway,
        SettingsError::RevertFailed { .. } => Status::Conflict,
        SettingsError::Device(proview_core::dvrip::client::DvrError::Rejected { ret, .. }) => {
            match ret {
                607 => Status::NotImplemented,
                103 | 107 => Status::Forbidden,
                _ => Status::BadGateway,
            }
        }
        SettingsError::Device(_) => Status::BadGateway,
    };
    fail(status, error.to_string())
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

pub struct DeviceLink<'r> {
    cfg: &'r Config,
    cloud: &'r Arc<CloudProvider>,
    modes: &'r ModeStore,
    store: &'r Store,
    cookies: &'r CookieJar<'r>,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for DeviceLink<'r> {
    type Error = &'static str;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let rocket = request.rocket();
        match (
            rocket.state::<Config>(),
            rocket.state::<Arc<CloudProvider>>(),
            rocket.state::<ModeStore>(),
            rocket.state::<Store>(),
        ) {
            (Some(cfg), Some(cloud), Some(modes), Some(store)) => Outcome::Success(DeviceLink {
                cfg,
                cloud,
                modes,
                store,
                cookies: request.cookies(),
            }),
            _ => Outcome::Error((
                Status::InternalServerError,
                "the device settings state is not managed",
            )),
        }
    }
}

impl DeviceLink<'_> {
    fn controls(&self, device: Option<&str>) -> Result<Controls, Failure> {
        controls_for(
            device,
            self.cfg,
            self.cloud,
            self.modes,
            self.store,
            self.cookies,
        )
    }

    async fn session(&self, device: Option<&str>, window: Duration) -> Result<Connection, Failure> {
        let controls = self.controls(device)?;
        open_session(&controls, window).await
    }
}

async fn open_session(controls: &Controls, window: Duration) -> Result<Connection, Failure> {
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

async fn finish(mut connection: Connection, window: Duration) {
    let _ = connection.logout_within(window).await;
}

pub(crate) async fn device_summary(controls: &Controls) -> Result<Value, Failure> {
    let window = settings_window();
    let mut connection = open_session(controls, window).await?;
    let system = tokio::time::timeout(window, connection.system_info()).await;
    let system = match system {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            finish(connection, window).await;
            return Err(settings_failure(SettingsError::Device(error)));
        }
        Err(_) => {
            return Err(settings_failure(SettingsError::Slow {
                block: "SystemInfo",
                detail: "the read never came".to_string(),
            }))
        }
    };
    let encode = connection
        .encode_state_within(window)
        .await
        .map_err(settings_failure)?;
    let network = connection
        .network_state_within(window)
        .await
        .map_err(settings_failure)?;
    let storage = connection
        .storage_state_within(window)
        .await
        .map_err(settings_failure)?;
    finish(connection, window).await;
    Ok(json!({
        "ok": true,
        "backend": "dvrip",
        "device": system,
        "encode": encode,
        "network": network,
        "storage": storage,
    }))
}

#[derive(Serialize)]
pub struct EncodeBody {
    pub ok: bool,
    pub backend: &'static str,
    pub encode: EncodeState,
    pub capability: Option<EncodeCapabilityView>,
    pub smart: Option<SmartEncodeState>,
}

#[get("/device/settings/encode?<device>")]
async fn encode_read(
    device: Option<String>,
    link: DeviceLink<'_>,
) -> Result<Json<EncodeBody>, Failure> {
    let window = settings_window();
    let mut connection = link.session(device.as_deref(), window).await?;
    let encode = connection
        .encode_state_within(window)
        .await
        .map_err(settings_failure)?;
    let capability =
        match tokio::time::timeout(window, connection.encode_capability_view(&encode)).await {
            Ok(view) => Some(view),
            Err(_) => {
                connection.discard_late_replies().await;
                None
            }
        };
    let smart = match tokio::time::timeout(window, connection.smart_encode_state()).await {
        Ok(result) => result.ok().flatten(),
        Err(_) => {
            connection.discard_late_replies().await;
            None
        }
    };
    finish(connection, window).await;
    Ok(Json(EncodeBody {
        ok: true,
        backend: "dvrip",
        encode,
        capability,
        smart,
    }))
}

#[derive(FromForm)]
pub struct EncodeQuery {
    device: Option<String>,
    stream: Option<String>,
    fps: Option<i64>,
    bitrate: Option<i64>,
    gop: Option<i64>,
    resolution: Option<String>,
    smart: Option<String>,
}

#[derive(Serialize, Default)]
pub struct WriteBody {
    pub ok: bool,
    pub backend: &'static str,
    pub reverted: bool,
    pub stream: Option<String>,
    pub reports: Vec<WriteReport>,
    pub errors: Vec<String>,
    pub encode: Option<EncodeState>,
    pub smart: Option<SmartEncodeState>,
    pub storage: Option<proview_core::dvrip::settings::StorageState>,
    pub record: Option<RecordState>,
    pub osd: Option<OsdState>,
}

impl WriteBody {
    fn settle(mut self) -> (Status, Json<WriteBody>) {
        self.backend = "dvrip";
        self.reverted = self.reports.iter().any(|report| report.reverted);
        self.ok = self.errors.is_empty() && !self.reverted;
        let status = if self.ok {
            Status::Ok
        } else {
            Status::Conflict
        };
        (status, Json(self))
    }
}

fn collect(
    outcome: Result<WriteReport, SettingsError>,
    reports: &mut Vec<WriteReport>,
    errors: &mut Vec<String>,
) -> Option<Failure> {
    match outcome {
        Ok(report) => {
            reports.push(report);
            None
        }
        Err(error) if reports.is_empty() && errors.is_empty() => Some(settings_failure(error)),
        Err(error) => {
            errors.push(error.to_string());
            None
        }
    }
}

#[rocket::put("/device/settings/encode?<query..>")]
async fn encode_write(
    query: EncodeQuery,
    link: DeviceLink<'_>,
) -> Result<(Status, Json<WriteBody>), Failure> {
    let stream = Stream::parse(query.stream.as_deref().unwrap_or("main"))
        .ok_or_else(|| fail(Status::BadRequest, "stream must be main or extra"))?;
    let smart =
        match query.smart.as_deref() {
            Some(raw) => Some(SmartMode::parse(raw).ok_or_else(|| {
                fail(Status::BadRequest, "smart must be off, smart or smart_plus")
            })?),
            None => None,
        };
    let patch = EncodePatch {
        fps: query.fps,
        bitrate: query.bitrate,
        gop: query.gop,
        resolution: query.resolution.clone(),
    };
    let video = patch.fps.is_some()
        || patch.bitrate.is_some()
        || patch.gop.is_some()
        || patch.resolution.is_some();
    if !video && smart.is_none() {
        return Err(fail(
            Status::BadRequest,
            "name at least one of fps, bitrate, gop, resolution or smart",
        ));
    }
    let write = write_window();
    let mut connection = link.session(query.device.as_deref(), write.timeout).await?;
    if let Some(mode) = smart {
        if let Err(error) = smart_preflight(&mut connection, mode, write.timeout).await {
            finish(connection, write.timeout).await;
            return Err(settings_failure(error));
        }
    }
    let mut body = WriteBody {
        stream: Some(stream.label().to_string()),
        ..WriteBody::default()
    };
    if video {
        let outcome = connection.write_encode(stream, &patch, &write).await;
        if let Some(failure) = collect(outcome, &mut body.reports, &mut body.errors) {
            finish(connection, write.timeout).await;
            return Err(failure);
        }
    }
    if let (Some(mode), false) = (smart, body.reverted_any()) {
        let outcome = connection.write_smart_encode(mode, &write).await;
        if let Some(failure) = collect(outcome, &mut body.reports, &mut body.errors) {
            finish(connection, write.timeout).await;
            return Err(failure);
        }
    }
    body.encode = connection.encode_state_within(write.timeout).await.ok();
    if smart.is_some() {
        body.smart = tokio::time::timeout(write.timeout, connection.smart_encode_state())
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
    }
    finish(connection, write.timeout).await;
    Ok(body.settle())
}

async fn smart_preflight(
    connection: &mut Connection,
    mode: SmartMode,
    window: Duration,
) -> Result<(), SettingsError> {
    let state = match tokio::time::timeout(window, connection.smart_encode_state()).await {
        Ok(result) => result?,
        Err(_) => {
            connection.discard_late_replies().await;
            return Err(SettingsError::Slow {
                block: proview_core::dvrip::encode_osd::SMART_V2_BLOCK,
                detail: "the smart encode read before the write never came".to_string(),
            });
        }
    };
    match state {
        Some(state) if state.modes.contains(&mode) => Ok(()),
        Some(state) => Err(SettingsError::Invalid {
            field: "smart",
            detail: format!(
                "{} is not offered by this device ({} reports {})",
                mode.label(),
                proview_core::dvrip::encode_osd::ENCODE_264_ABILITY,
                state
                    .modes
                    .iter()
                    .map(|mode| mode.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }),
        None => Err(SettingsError::Missing {
            block: proview_core::dvrip::encode_osd::SMART_V2_BLOCK,
            path: format!(
                "{}, {} and {}",
                proview_core::dvrip::encode_osd::SMART_V2_BLOCK,
                proview_core::dvrip::encode_osd::ENCODE_264_ABILITY,
                proview_core::dvrip::encode_osd::SMART_BLOCK
            ),
        }),
    }
}

#[derive(Serialize)]
pub struct NetworkBody {
    pub ok: bool,
    pub backend: &'static str,
    pub network: proview_core::dvrip::settings::NetworkState,
}

#[get("/device/settings/network?<device>")]
async fn network_read(
    device: Option<String>,
    link: DeviceLink<'_>,
) -> Result<Json<NetworkBody>, Failure> {
    let window = settings_window();
    let mut connection = link.session(device.as_deref(), window).await?;
    let network = connection
        .network_state_within(window)
        .await
        .map_err(settings_failure)?;
    finish(connection, window).await;
    Ok(Json(NetworkBody {
        ok: true,
        backend: "dvrip",
        network,
    }))
}

#[derive(FromForm)]
pub struct NetworkQuery {
    device: Option<String>,
    dhcp: Option<bool>,
    dns: Option<String>,
    spare_dns: Option<String>,
    gateway: Option<String>,
    submask: Option<String>,
    ip: Option<String>,
}

#[rocket::put("/device/settings/network?<query..>")]
async fn network_write(
    query: NetworkQuery,
    link: DeviceLink<'_>,
) -> Result<(Status, Json<WriteBody>), Failure> {
    let moved = [
        ("ip", query.ip.as_deref()),
        (
            "dhcp",
            query.dhcp.map(|value| if value { "on" } else { "off" }),
        ),
        ("gateway", query.gateway.as_deref()),
        ("submask", query.submask.as_deref()),
    ]
    .into_iter()
    .filter_map(|(field, value)| {
        value
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!("{field}={value}"))
    })
    .collect::<Vec<_>>();
    if !moved.is_empty() {
        return Err(fail(
            Status::Conflict,
            format!(
                "{} would change how this app reaches the camera, and both the read-back and the automatic revert run over that path; the write is refused and the device keeps its route (DNS stays writable)",
                moved.join(", ")
            ),
        ));
    }
    let patch = NetworkPatch {
        dns: query.dns.clone(),
        spare_dns: query.spare_dns.clone(),
    };
    if patch.empty() {
        return Err(fail(
            Status::BadRequest,
            "name dns and/or spare_dns; the address, dhcp, gateway and submask are refused because the revert travels over them",
        ));
    }
    let write = write_window();
    let mut connection = link.session(query.device.as_deref(), write.timeout).await?;
    let reports = connection
        .write_network(&patch, &write)
        .await
        .map_err(settings_failure)?;
    finish(connection, write.timeout).await;
    Ok(WriteBody {
        reports,
        ..WriteBody::default()
    }
    .settle())
}

#[derive(Serialize)]
pub struct StorageBody {
    pub ok: bool,
    pub backend: &'static str,
    pub storage: proview_core::dvrip::settings::StorageState,
}

#[get("/device/settings/storage?<device>")]
async fn storage_read(
    device: Option<String>,
    link: DeviceLink<'_>,
) -> Result<Json<StorageBody>, Failure> {
    let window = settings_window();
    let mut connection = link.session(device.as_deref(), window).await?;
    let storage = connection
        .storage_state_within(window)
        .await
        .map_err(settings_failure)?;
    finish(connection, window).await;
    Ok(Json(StorageBody {
        ok: true,
        backend: "dvrip",
        storage,
    }))
}

#[derive(FromForm)]
pub struct StorageQuery {
    device: Option<String>,
    overwrite: Option<String>,
    record: Option<String>,
    missing_alarm: Option<bool>,
}

#[rocket::put("/device/settings/storage?<query..>")]
async fn storage_write(
    query: StorageQuery,
    link: DeviceLink<'_>,
) -> Result<(Status, Json<WriteBody>), Failure> {
    let overwrite = match query.overwrite.as_deref() {
        Some(raw) => Some(
            OverwriteMode::parse(raw)
                .ok_or_else(|| fail(Status::BadRequest, "overwrite must be overwrite or stop"))?,
        ),
        None => None,
    };
    let record = match query.record.as_deref() {
        Some(raw) => Some(
            RecordMode::parse(raw)
                .ok_or_else(|| fail(Status::BadRequest, "record must be config, manual or off"))?,
        ),
        None => None,
    };
    if overwrite.is_none() && record.is_none() && query.missing_alarm.is_none() {
        return Err(fail(
            Status::BadRequest,
            "name overwrite, record and/or missing_alarm",
        ));
    }
    let write = write_window();
    let mut connection = link.session(query.device.as_deref(), write.timeout).await?;
    let mut body = WriteBody::default();
    if let Some(mode) = overwrite {
        match connection.write_overwrite(mode, &write).await {
            Ok(report) => body.reports.push(report),
            Err(error) => body.errors.push(error.to_string()),
        }
    }
    if let Some(mode) = record {
        match connection.write_record_mode(mode, &write).await {
            Ok(report) => body.reports.push(report),
            Err(error) => body.errors.push(error.to_string()),
        }
    }
    if let Some(enable) = query.missing_alarm {
        match connection.write_storage_missing_alarm(enable, &write).await {
            Ok(report) => body.reports.push(report),
            Err(error) => body.errors.push(error.to_string()),
        }
    }
    if body.errors.is_empty() {
        body.storage = connection.storage_state_within(write.timeout).await.ok();
    }
    finish(connection, write.timeout).await;
    Ok(body.settle())
}

#[derive(Serialize)]
pub struct FormatBody {
    pub ok: bool,
    pub backend: &'static str,
    pub steps: Vec<FormatStep>,
    pub storage: Option<proview_core::dvrip::settings::StorageState>,
}

#[rocket::post("/device/settings/storage/format?<device>&<confirm>")]
async fn storage_format(
    device: Option<String>,
    confirm: Option<String>,
    link: DeviceLink<'_>,
) -> Result<Json<FormatBody>, Failure> {
    link.controls(device.as_deref())?;
    if confirm.as_deref().map(str::trim) != Some(FORMAT_CONFIRMATION) {
        return Err(fail(
            Status::BadRequest,
            format!("formatting erases every recording on the card; send confirm={FORMAT_CONFIRMATION} to proceed"),
        ));
    }
    let window = format_window();
    let mut connection = link.session(device.as_deref(), settings_window()).await?;
    let steps = connection
        .format_card(window)
        .await
        .map_err(settings_failure)?;
    let storage = connection
        .storage_state_within(settings_window())
        .await
        .ok();
    finish(connection, settings_window()).await;
    Ok(Json(FormatBody {
        ok: true,
        backend: "dvrip",
        steps,
        storage,
    }))
}

#[derive(Serialize)]
pub struct RecordBody {
    pub ok: bool,
    pub backend: &'static str,
    pub record: RecordState,
}

#[get("/device/settings/record?<device>")]
async fn record_read(
    device: Option<String>,
    link: DeviceLink<'_>,
) -> Result<Json<RecordBody>, Failure> {
    let window = settings_window();
    let mut connection = link.session(device.as_deref(), window).await?;
    let record = connection
        .record_state_within(window)
        .await
        .map_err(settings_failure)?;
    finish(connection, window).await;
    Ok(Json(RecordBody {
        ok: true,
        backend: "dvrip",
        record,
    }))
}

#[derive(FromForm)]
pub struct RecordQuery {
    device: Option<String>,
    full_time: Option<bool>,
    packet_length: Option<i64>,
    record_stream: Option<String>,
    splicing: Option<bool>,
    audio: Option<bool>,
}

#[rocket::put("/device/settings/record?<query..>")]
async fn record_write(
    query: RecordQuery,
    link: DeviceLink<'_>,
) -> Result<(Status, Json<WriteBody>), Failure> {
    let record_stream = match query.record_stream.as_deref() {
        Some(raw) => Some(
            Stream::parse(raw)
                .ok_or_else(|| fail(Status::BadRequest, "record_stream must be main or extra"))?,
        ),
        None => None,
    };
    let schedule = query.full_time.is_some() || query.packet_length.is_some();
    if !schedule && record_stream.is_none() && query.splicing.is_none() && query.audio.is_none() {
        return Err(fail(
            Status::BadRequest,
            "name full_time, packet_length, record_stream, splicing and/or audio",
        ));
    }
    if let Some(minutes) = query.packet_length {
        check_packet_length(minutes).map_err(settings_failure)?;
    }
    let write = write_window();
    let mut connection = link.session(query.device.as_deref(), write.timeout).await?;
    if let Err(error) = connection
        .record_preflight(
            query.full_time,
            record_stream.is_some(),
            query.splicing.is_some(),
            write.timeout,
        )
        .await
    {
        finish(connection, write.timeout).await;
        return Err(settings_failure(error));
    }
    let mut body = WriteBody::default();
    for step in 0..4 {
        let outcome = match step {
            0 if schedule => {
                connection
                    .write_record_schedule(query.full_time, query.packet_length, &write)
                    .await
            }
            1 if record_stream.is_some() => {
                let stream = record_stream.unwrap_or(Stream::Main);
                connection.write_record_stream(stream, &write).await
            }
            2 if query.splicing.is_some() => {
                let enable = query.splicing.unwrap_or(false);
                connection.write_splicing(enable, &write).await
            }
            3 if query.audio.is_some() => {
                let enable = query.audio.unwrap_or(false);
                connection.write_record_audio(enable, &write).await
            }
            _ => continue,
        };
        if let Some(failure) = collect(outcome, &mut body.reports, &mut body.errors) {
            finish(connection, write.timeout).await;
            return Err(failure);
        }
        if !body.errors.is_empty() || body.reverted_any() {
            break;
        }
    }
    body.record = connection.record_state_within(write.timeout).await.ok();
    finish(connection, write.timeout).await;
    Ok(body.settle())
}

#[derive(Serialize)]
pub struct OsdBody {
    pub ok: bool,
    pub backend: &'static str,
    pub osd: OsdState,
}

#[get("/device/settings/osd?<device>&<channel>")]
async fn osd_read(
    device: Option<String>,
    channel: Option<usize>,
    link: DeviceLink<'_>,
) -> Result<Json<OsdBody>, Failure> {
    let window = settings_window();
    let mut connection = link.session(device.as_deref(), window).await?;
    let osd = connection
        .osd_state_within(channel.unwrap_or(0), window)
        .await
        .map_err(settings_failure)?;
    finish(connection, window).await;
    Ok(Json(OsdBody {
        ok: true,
        backend: "dvrip",
        osd,
    }))
}

#[derive(FromForm)]
pub struct OsdQuery {
    device: Option<String>,
    channel: Option<usize>,
    name: Option<String>,
    title: Option<bool>,
    time: Option<bool>,
}

#[rocket::put("/device/settings/osd?<query..>")]
async fn osd_write(
    query: OsdQuery,
    link: DeviceLink<'_>,
) -> Result<(Status, Json<WriteBody>), Failure> {
    let name = match query.name.as_deref() {
        Some(raw) => {
            Some(proview_core::dvrip::encode_osd::check_title(raw).map_err(settings_failure)?)
        }
        None => None,
    };
    let patch = OsdPatch {
        channel: query.channel.unwrap_or(0),
        name: name.clone(),
        title_shown: query.title,
        time_shown: query.time,
    };
    if patch.name.is_none() && patch.title_shown.is_none() && patch.time_shown.is_none() {
        return Err(fail(
            Status::BadRequest,
            "name at least one of name, title or time",
        ));
    }
    let write = write_window();
    let mut connection = link.session(query.device.as_deref(), write.timeout).await?;
    let mut body = WriteBody::default();
    let outcome = connection.write_video_widget(&patch, &write).await;
    if let Some(failure) = collect(outcome, &mut body.reports, &mut body.errors) {
        finish(connection, write.timeout).await;
        return Err(failure);
    }
    if let (Some(name), false) = (name.as_deref(), body.reverted_any()) {
        let outcome = connection
            .write_channel_title(patch.channel, name, &write)
            .await;
        if let Some(failure) = collect(outcome, &mut body.reports, &mut body.errors) {
            finish(connection, write.timeout).await;
            return Err(failure);
        }
    }
    body.osd = connection
        .osd_state_within(patch.channel, write.timeout)
        .await
        .ok();
    finish(connection, write.timeout).await;
    Ok(body.settle())
}

impl WriteBody {
    fn reverted_any(&self) -> bool {
        self.reports.iter().any(|report| report.reverted)
    }
}

#[derive(Deserialize)]
pub struct TitleDotBody {
    width: usize,
    height: usize,
    bits: String,
}

fn hex_bytes(raw: &str) -> Option<Vec<u8>> {
    let raw = raw.trim();
    if raw.len() % 2 != 0 {
        return None;
    }
    (0..raw.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(raw.get(index..index + 2)?, 16).ok())
        .collect()
}

#[derive(Serialize)]
pub struct TitleDotReply {
    pub ok: bool,
    pub backend: &'static str,
    pub ret: i64,
    pub width: usize,
    pub height: usize,
    pub bytes: usize,
}

#[rocket::post("/device/settings/osd/dot?<device>", data = "<dot>")]
async fn osd_dot(
    device: Option<String>,
    dot: Json<TitleDotBody>,
    link: DeviceLink<'_>,
) -> Result<Json<TitleDotReply>, Failure> {
    let bits = hex_bytes(&dot.bits)
        .ok_or_else(|| fail(Status::BadRequest, "bits must be an even-length hex string"))?;
    proview_core::dvrip::encode_osd::title_dot_frame(dot.width, dot.height, &bits)
        .map_err(settings_failure)?;
    let window = settings_window();
    let mut connection = link.session(device.as_deref(), window).await?;
    let ret = connection
        .write_title_dot(dot.width, dot.height, &bits, window)
        .await
        .map_err(settings_failure)?;
    finish(connection, window).await;
    Ok(Json(TitleDotReply {
        ok: true,
        backend: "dvrip",
        ret,
        width: dot.width,
        height: dot.height,
        bytes: bits.len(),
    }))
}
