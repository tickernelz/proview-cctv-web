use std::path::PathBuf;
use std::time::Duration;

use proview_core::config::Config;
use proview_core::device::{ControlError, Controls, SessionError};
use proview_core::dvrip::control::RecordMode;
use proview_core::dvrip::settings::{
    EncodePatch, NetworkPatch, OverwriteMode, SettingsError, Stream, WriteWindow,
};
use proview_core::session::Store;
use rocket::form::FromForm;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;

use super::{fail, session_id, Failure};

pub fn routes() -> Vec<rocket::route::Route> {
    routes![
        encode_read,
        encode_write,
        network_read,
        network_write,
        storage_read,
        storage_write,
    ]
}

const BACKUP_ENV: &str = "PROVIEW_SETTINGS_BACKUP_DIR";
const WINDOW_ENV: &str = "PROVIEW_SETTINGS_WINDOW_MS";
const DEFAULT_WINDOW: Duration = Duration::from_secs(20);

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

fn settings_window() -> Duration {
    match std::env::var(WINDOW_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        Some(ms) if ms > 0 => Duration::from_millis(ms),
        _ => DEFAULT_WINDOW,
    }
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

fn signed_in(store: &State<Store>, cookies: &CookieJar<'_>) -> Result<(), Failure> {
    let id = session_id(cookies)?;
    store
        .get(&id)
        .map(|_| ())
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))
}

async fn session(
    cfg: &State<Config>,
    window: Duration,
) -> Result<proview_core::dvrip::client::Connection, Failure> {
    let controls = Controls::from_config(cfg);
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

#[derive(Serialize)]
pub struct EncodeBody {
    pub ok: bool,
    pub backend: &'static str,
    pub encode: proview_core::dvrip::settings::EncodeState,
}

#[get("/device/settings/encode")]
async fn encode_read(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<EncodeBody>, Failure> {
    signed_in(store, cookies)?;
    let window = settings_window();
    let mut connection = session(cfg, window).await?;
    let encode = connection
        .encode_state_within(window)
        .await
        .map_err(settings_failure)?;
    let _ = connection.logout_within(window).await;
    Ok(Json(EncodeBody {
        ok: true,
        backend: "dvrip",
        encode,
    }))
}

#[derive(FromForm)]
pub struct EncodeQuery {
    stream: Option<String>,
    fps: Option<i64>,
    bitrate: Option<i64>,
    gop: Option<i64>,
    resolution: Option<String>,
}

#[derive(Serialize)]
pub struct WriteBody {
    pub ok: bool,
    pub backend: &'static str,
    pub reverted: bool,
    pub stream: Option<String>,
    pub reports: Vec<proview_core::dvrip::settings::WriteReport>,
    pub errors: Vec<String>,
    pub encode: Option<proview_core::dvrip::settings::EncodeState>,
    pub storage: Option<proview_core::dvrip::settings::StorageState>,
}

#[rocket::put("/device/settings/encode?<query..>")]
async fn encode_write(
    query: EncodeQuery,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<(Status, Json<WriteBody>), Failure> {
    signed_in(store, cookies)?;
    let stream = Stream::parse(query.stream.as_deref().unwrap_or("main"))
        .ok_or_else(|| fail(Status::BadRequest, "stream must be main or extra"))?;
    let patch = EncodePatch {
        fps: query.fps,
        bitrate: query.bitrate,
        gop: query.gop,
        resolution: query.resolution.clone(),
    };
    let write = write_window();
    let mut connection = session(cfg, write.timeout).await?;
    let report = connection
        .write_encode(stream, &patch, &write)
        .await
        .map_err(settings_failure)?;
    let encode = connection.encode_state_within(write.timeout).await.ok();
    let _ = connection.logout_within(write.timeout).await;
    let reverted = report.reverted;
    let body = WriteBody {
        ok: !reverted,
        backend: "dvrip",
        reverted,
        stream: Some(stream.label().to_string()),
        reports: vec![report],
        errors: Vec::new(),
        encode,
        storage: None,
    };
    let status = if reverted {
        Status::Conflict
    } else {
        Status::Ok
    };
    Ok((status, Json(body)))
}

#[derive(Serialize)]
pub struct NetworkBody {
    pub ok: bool,
    pub backend: &'static str,
    pub network: proview_core::dvrip::settings::NetworkState,
}

#[get("/device/settings/network")]
async fn network_read(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<NetworkBody>, Failure> {
    signed_in(store, cookies)?;
    let window = settings_window();
    let mut connection = session(cfg, window).await?;
    let network = connection
        .network_state_within(window)
        .await
        .map_err(settings_failure)?;
    let _ = connection.logout_within(window).await;
    Ok(Json(NetworkBody {
        ok: true,
        backend: "dvrip",
        network,
    }))
}

#[derive(FromForm)]
pub struct NetworkQuery {
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
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<(Status, Json<WriteBody>), Failure> {
    signed_in(store, cookies)?;
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
    let mut connection = session(cfg, write.timeout).await?;
    let reports = connection
        .write_network(&patch, &write)
        .await
        .map_err(settings_failure)?;
    let _ = connection.logout_within(write.timeout).await;
    let reverted = reports.iter().any(|report| report.reverted);
    let errors = Vec::new();
    let status = if reverted {
        Status::Conflict
    } else {
        Status::Ok
    };
    Ok((
        status,
        Json(WriteBody {
            ok: !reverted,
            backend: "dvrip",
            reverted,
            stream: None,
            reports,
            errors,
            encode: None,
            storage: None,
        }),
    ))
}

#[derive(Serialize)]
pub struct StorageBody {
    pub ok: bool,
    pub backend: &'static str,
    pub storage: proview_core::dvrip::settings::StorageState,
}

#[get("/device/settings/storage")]
async fn storage_read(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StorageBody>, Failure> {
    signed_in(store, cookies)?;
    let window = settings_window();
    let mut connection = session(cfg, window).await?;
    let storage = connection
        .storage_state_within(window)
        .await
        .map_err(settings_failure)?;
    let _ = connection.logout_within(window).await;
    Ok(Json(StorageBody {
        ok: true,
        backend: "dvrip",
        storage,
    }))
}

#[derive(FromForm)]
pub struct StorageQuery {
    overwrite: Option<String>,
    record: Option<String>,
}

#[rocket::put("/device/settings/storage?<query..>")]
async fn storage_write(
    query: StorageQuery,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<(Status, Json<WriteBody>), Failure> {
    signed_in(store, cookies)?;
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
    if overwrite.is_none() && record.is_none() {
        return Err(fail(Status::BadRequest, "name overwrite and/or record"));
    }
    let write = write_window();
    let mut connection = session(cfg, write.timeout).await?;
    let mut reports = Vec::new();
    let mut errors = Vec::new();
    if let Some(mode) = overwrite {
        match connection.write_overwrite(mode, &write).await {
            Ok(report) => reports.push(report),
            Err(error) => errors.push(error.to_string()),
        }
    }
    if let Some(mode) = record {
        match connection.write_record_mode(mode, &write).await {
            Ok(report) => reports.push(report),
            Err(error) => errors.push(error.to_string()),
        }
    }
    let storage = if errors.is_empty() {
        let storage = connection.storage_state_within(write.timeout).await.ok();
        let _ = connection.logout_within(write.timeout).await;
        storage
    } else {
        None
    };
    let reverted = reports.iter().any(|report| report.reverted);
    let status = if errors.is_empty() && !reverted {
        Status::Ok
    } else {
        Status::Conflict
    };
    let ok = errors.is_empty() && !reverted;
    Ok((
        status,
        Json(WriteBody {
            ok,
            backend: "dvrip",
            reverted,
            stream: None,
            reports,
            errors,
            encode: None,
            storage,
        }),
    ))
}
