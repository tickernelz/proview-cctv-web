use std::path::PathBuf;

use proview_core::config::Config;
use proview_core::device::{ControlError, Controls};
use proview_core::dvrip::control::RecordMode;
use proview_core::dvrip::settings::{
    EncodePatch, NetworkPatch, OverwriteMode, SettingsError, Stream,
};
use proview_core::session::Store;
use rocket::form::FromForm;
use rocket::http::{CookieJar, Status};
use rocket::response::content::RawJavaScript;
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;

use super::{fail, session_id, Failure};

pub fn routes() -> Vec<rocket::route::Route> {
    routes![
        device_settings_js,
        encode_read,
        encode_write,
        network_read,
        network_write,
        storage_read,
        storage_write,
    ]
}

const BACKUP_ENV: &str = "PROVIEW_SETTINGS_BACKUP_DIR";

fn backup_dir() -> PathBuf {
    let explicit = std::env::var(BACKUP_ENV).ok().filter(|value| !value.trim().is_empty());
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

fn settings_failure(error: SettingsError) -> Failure {
    let status = match &error {
        SettingsError::Invalid { .. } => Status::BadRequest,
        SettingsError::Missing { .. } => Status::NotImplemented,
        SettingsError::Backup { .. } => Status::InternalServerError,
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
) -> Result<proview_core::dvrip::client::Connection, Failure> {
    let controls = Controls::from_config(cfg);
    let control = controls.dvrip().map_err(control_failure)?;
    control.session().await.map_err(control_failure)
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
    let mut connection = session(cfg).await?;
    let encode = connection
        .encode_state()
        .await
        .map_err(settings_failure)?;
    let _ = connection.logout().await;
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
    let mut connection = session(cfg).await?;
    let report = connection
        .write_encode(stream, &patch, &backup_dir())
        .await
        .map_err(settings_failure)?;
    let encode = connection.encode_state().await.ok();
    let _ = connection.logout().await;
    let reverted = report.reverted;
    let body = WriteBody {
        ok: !reverted,
        backend: "dvrip",
        reverted,
        stream: Some(stream.label().to_string()),
        reports: vec![report],
        encode,
        storage: None,
    };
    let status = if reverted { Status::Conflict } else { Status::Ok };
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
    let mut connection = session(cfg).await?;
    let network = connection
        .network_state()
        .await
        .map_err(settings_failure)?;
    let _ = connection.logout().await;
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
    if let Some(ip) = query.ip.as_deref().filter(|value| !value.trim().is_empty()) {
        return Err(fail(
            Status::Conflict,
            format!(
                "a new address ({ip}) would move the camera off the address this app reaches; the read-back and the automatic revert both run over that address, so the write is refused and the device keeps its address"
            ),
        ));
    }
    let patch = NetworkPatch {
        dhcp: query.dhcp,
        dns: query.dns.clone(),
        spare_dns: query.spare_dns.clone(),
        gateway: query.gateway.clone(),
        submask: query.submask.clone(),
    };
    if patch.empty() {
        return Err(fail(
            Status::BadRequest,
            "name at least one of dhcp, dns, spare_dns, gateway or submask",
        ));
    }
    let mut connection = session(cfg).await?;
    let reports = connection
        .write_network(&patch, &backup_dir())
        .await
        .map_err(settings_failure)?;
    let _ = connection.logout().await;
    let reverted = reports.iter().any(|report| report.reverted);
    let status = if reverted { Status::Conflict } else { Status::Ok };
    Ok((
        status,
        Json(WriteBody {
            ok: !reverted,
            backend: "dvrip",
            reverted,
            stream: None,
            reports,
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
    let mut connection = session(cfg).await?;
    let storage = connection
        .storage_state()
        .await
        .map_err(settings_failure)?;
    let _ = connection.logout().await;
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
        return Err(fail(
            Status::BadRequest,
            "name overwrite and/or record",
        ));
    }
    let mut connection = session(cfg).await?;
    let mut reports = Vec::new();
    if let Some(mode) = overwrite {
        reports.push(
            connection
                .write_overwrite(mode, &backup_dir())
                .await
                .map_err(settings_failure)?,
        );
    }
    if let Some(mode) = record {
        reports.push(
            connection
                .write_record_mode(mode, &backup_dir())
                .await
                .map_err(settings_failure)?,
        );
    }
    let storage = connection.storage_state().await.ok();
    let _ = connection.logout().await;
    let reverted = reports.iter().any(|report| report.reverted);
    let status = if reverted { Status::Conflict } else { Status::Ok };
    Ok((
        status,
        Json(WriteBody {
            ok: !reverted,
            backend: "dvrip",
            reverted,
            stream: None,
            reports,
            encode: None,
            storage,
        }),
    ))
}

#[get("/device-settings.js")]
pub fn device_settings_js() -> RawJavaScript<&'static str> {
    RawJavaScript(include_str!("../../../../web/device-settings.js"))
}
