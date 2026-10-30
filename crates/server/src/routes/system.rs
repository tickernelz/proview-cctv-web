use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use proview_core::cloud::upgrade::{self, ServerCheck, UpgradeClient, UpgradeQuery};
use proview_core::config::Config;
use proview_core::device::{ControlError, Controls, SessionError};
use proview_core::dvrip::client::{Connection, DvrError};
use proview_core::dvrip::system::{self, DeviceUser, PingReport, RebootAck, PING_HOSTS};
use proview_core::provider::cloud::CloudProvider;
use proview_core::session::Store;
use rocket::data::{Data, ToByteUnit};
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::control::controls_for;
use super::{fail, Failure};
use crate::config::ModeStore;

pub fn routes() -> Vec<rocket::route::Route> {
    routes![
        about,
        time_read,
        time_write,
        language_read,
        language_write,
        reboot,
        defaults,
        firmware_check,
        firmware_start,
        firmware_file,
        network_check,
        users_read,
        users_password,
        users_rename,
        users_recover,
    ]
}

const WINDOW_ENV: &str = "PROVIEW_SYSTEM_WINDOW_MS";
const DEFAULT_WINDOW: Duration = Duration::from_secs(20);
const MAX_FIRMWARE: usize = 64 * 1024 * 1024;
const RESET_WORD: &str = "RESET";
const UPGRADE_WORD: &str = "UPGRADE";
const UPGRADE_LANGUAGE: &str = "English";
const REBOOT_HOLD: Duration = Duration::from_secs(5);

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
    std::env::var("PROVIEW_DATA_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("system-backups")
}

fn save_backup(name: &str, value: &Value) -> Result<String, Failure> {
    let dir = backup_dir();
    std::fs::create_dir_all(&dir).map_err(|error| {
        fail(
            Status::InternalServerError,
            format!("could not create {}: {error}", dir.display()),
        )
    })?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("{stamp}-{name}.json"));
    std::fs::write(&path, value.to_string()).map_err(|error| {
        fail(
            Status::InternalServerError,
            format!(
                "could not save the original {name} to {}: {error}",
                path.display()
            ),
        )
    })?;
    Ok(path.display().to_string())
}

fn device_failure(stage: &str, error: DvrError) -> Failure {
    let status = match &error {
        DvrError::Rejected { ret, .. } => match ret {
            502 | 605 | 607 => Status::NotImplemented,
            103 | 107 => Status::Forbidden,
            _ => Status::BadGateway,
        },
        _ => Status::BadGateway,
    };
    fail(status, format!("{stage}: {error}"))
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

async fn within<T, F>(stage: &str, limit: Duration, work: F) -> Result<T, Failure>
where
    F: Future<Output = Result<T, DvrError>>,
{
    match tokio::time::timeout(limit, work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(device_failure(stage, error)),
        Err(_) => Err(fail(
            Status::GatewayTimeout,
            format!("{stage} did not answer inside {limit:?}"),
        )),
    }
}

async fn open(controls: &Controls, limit: Duration) -> Result<Connection, Failure> {
    let control = controls.dvrip().map_err(control_failure)?;
    control
        .session_within(limit)
        .await
        .map_err(|error| match error {
            SessionError::Timeout { address, window } => fail(
                Status::GatewayTimeout,
                format!("login: connect and login to {address} did not answer inside {window:?}"),
            ),
            SessionError::Failed(error) => control_failure(error),
        })
}

struct Link<'a> {
    device: Option<&'a str>,
    cfg: &'a Config,
    cloud: &'a Arc<CloudProvider>,
    modes: &'a ModeStore,
    store: &'a Store,
}

impl Link<'_> {
    async fn connect(
        &self,
        cookies: &CookieJar<'_>,
        limit: Duration,
    ) -> Result<Connection, Failure> {
        let controls = controls_for(
            self.device,
            self.cfg,
            self.cloud,
            self.modes,
            self.store,
            cookies,
        )?;
        open(&controls, limit).await
    }
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

#[derive(Serialize)]
pub struct AboutSummary {
    pub model: Option<String>,
    pub serial: Option<String>,
    pub firmware: Option<String>,
    pub build_time: Option<String>,
    pub hardware: Option<String>,
    pub uptime_minutes: Option<u64>,
}

fn summary(info: &Value) -> AboutSummary {
    AboutSummary {
        model: str_field(info, "DeviceModel").or_else(|| str_field(info, "HardWare")),
        serial: str_field(info, "SerialNo"),
        firmware: str_field(info, "SoftWareVersion"),
        build_time: str_field(info, "BuildTime"),
        hardware: str_field(info, "HardWare"),
        uptime_minutes: info.get("DeviceRunTime").and_then(system::uptime_minutes),
    }
}

#[derive(Serialize)]
pub struct Gates {
    pub cloud_upgrade: bool,
    pub wifi_signal: bool,
    pub mcu_version: bool,
}

fn gates(abilities: &Value) -> Gates {
    Gates {
        cloud_upgrade: system::ability(abilities, "OtherFunction/SupportCfgCloudupgrade"),
        wifi_signal: system::ability(abilities, "NetServerFunction/WifiRouteSignalLevel"),
        mcu_version: system::ability(abilities, "OtherFunction/SupportGetMcuVersion"),
    }
}

#[derive(Serialize)]
pub struct AboutBody {
    pub ok: bool,
    pub summary: AboutSummary,
    pub info: Value,
    pub info_ex: Option<Value>,
    pub info_ex_error: Option<String>,
    pub abilities: Vec<String>,
    pub gates: Gates,
    pub elapsed_ms: u64,
}

#[get("/device/system?<device>")]
async fn about(
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<AboutBody>, Failure> {
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let info = within("SystemInfo", limit, connection.system_info()).await?;
    let abilities = within("SystemFunction", limit, connection.abilities()).await?;
    let (info_ex, info_ex_error) = match within(
        "SystemInfoEx",
        limit,
        connection.system_entry("SystemInfoEx"),
    )
    .await
    {
        Ok(value) if !value.is_null() => (Some(value), None),
        Ok(_) => (
            None,
            Some("SystemInfoEx: the device sent an empty block".to_string()),
        ),
        Err((_, body)) => (None, Some(body.message.clone())),
    };
    let _ = connection.logout_within(limit).await;
    Ok(Json(AboutBody {
        ok: true,
        summary: summary(&info),
        info,
        info_ex,
        info_ex_error,
        abilities: system::ability_flags(&abilities),
        gates: gates(&abilities),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[derive(Serialize)]
pub struct TimeBody {
    pub ok: bool,
    pub device_time: Option<String>,
    pub minutes_east: Option<i64>,
    pub time_zone: Value,
    pub location: Value,
    pub backup: Option<String>,
    pub before: Option<Value>,
    pub elapsed_ms: u64,
}

async fn read_time(
    connection: &mut Connection,
    limit: Duration,
) -> Result<(Option<String>, Value, Value), Failure> {
    let device_time = within("OPTimeQuery", limit, connection.time_query()).await?;
    let zone = within(
        "System.TimeZone",
        limit,
        connection.config_get("System.TimeZone"),
    )
    .await?;
    let location = within(
        "General.Location",
        limit,
        connection.config_get("General.Location"),
    )
    .await?;
    Ok((
        device_time,
        zone.get("System.TimeZone").cloned().unwrap_or(Value::Null),
        location
            .get("General.Location")
            .cloned()
            .unwrap_or(Value::Null),
    ))
}

#[get("/device/system/time?<device>")]
async fn time_read(
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<TimeBody>, Failure> {
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let (device_time, time_zone, location) = read_time(&mut connection, limit).await?;
    let _ = connection.logout_within(limit).await;
    Ok(Json(TimeBody {
        ok: true,
        device_time,
        minutes_east: system::minutes_east_of_utc(&time_zone),
        time_zone,
        location,
        backup: None,
        before: None,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[post("/device/system/time?<device>&<time>&<offset>")]
#[allow(clippy::too_many_arguments)]
async fn time_write(
    device: Option<String>,
    time: Option<String>,
    offset: Option<i64>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<TimeBody>, Failure> {
    let started = Instant::now();
    if let Some(time) = time.as_deref() {
        if !system::valid_device_time(time) {
            return Err(fail(
                Status::BadRequest,
                "time must read YYYY-MM-DD HH:MM:SS in the camera's local time",
            ));
        }
    }
    if let Some(offset) = offset {
        if !(-14 * 60..=14 * 60).contains(&offset) {
            return Err(fail(
                Status::BadRequest,
                "offset is minutes east of UTC between -840 and 840",
            ));
        }
    }
    if time.is_none() && offset.is_none() {
        return Err(fail(Status::BadRequest, "pass time, offset or both"));
    }
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let (before_time, before_zone, _) = read_time(&mut connection, limit).await?;
    let before = json!({ "device_time": before_time, "time_zone": before_zone });
    let backup = save_backup("time", &before)?;
    if let Some(offset) = offset {
        if system::minutes_east_of_utc(&before_zone) != Some(offset) {
            within(
                "System.TimeZone write",
                limit,
                connection.config_set("System.TimeZone", system::time_zone_value(offset)),
            )
            .await?;
        }
    }
    if let Some(time) = time.as_deref() {
        within("OPTimeSetting", limit, connection.set_time(time)).await?;
    }
    let (device_time, time_zone, location) = read_time(&mut connection, limit).await?;
    let _ = connection.logout_within(limit).await;
    if let Some(offset) = offset {
        if system::minutes_east_of_utc(&time_zone) != Some(offset) {
            return Err(fail(
                Status::BadGateway,
                format!(
                    "System.TimeZone read back {} after writing {offset}; the original is saved in {backup}",
                    time_zone
                ),
            ));
        }
    }
    Ok(Json(TimeBody {
        ok: true,
        device_time,
        minutes_east: system::minutes_east_of_utc(&time_zone),
        time_zone,
        location,
        backup: Some(backup),
        before: Some(before),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[derive(Serialize)]
pub struct LanguageBody {
    pub ok: bool,
    pub current: Option<String>,
    pub available: Vec<String>,
    pub before: Option<String>,
    pub backup: Option<String>,
    pub elapsed_ms: u64,
}

async fn read_language(
    connection: &mut Connection,
    limit: Duration,
) -> Result<(Option<String>, Vec<String>), Failure> {
    let current = within(
        "General.Location.Language",
        limit,
        connection.config_get("General.Location.Language"),
    )
    .await?;
    let available = within(
        "MultiLanguage",
        limit,
        connection.ability_entry("MultiLanguage"),
    )
    .await?;
    Ok((
        current
            .get("General.Location.Language")
            .and_then(Value::as_str)
            .map(str::to_string),
        available
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    ))
}

#[get("/device/system/language?<device>")]
async fn language_read(
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<LanguageBody>, Failure> {
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let (current, available) = read_language(&mut connection, limit).await?;
    let _ = connection.logout_within(limit).await;
    Ok(Json(LanguageBody {
        ok: true,
        current,
        available,
        before: None,
        backup: None,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[put("/device/system/language?<device>&<value>")]
#[allow(clippy::too_many_arguments)]
async fn language_write(
    device: Option<String>,
    value: String,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<LanguageBody>, Failure> {
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let (before, available) = read_language(&mut connection, limit).await?;
    if !available.iter().any(|item| item == &value) {
        let _ = connection.logout_within(limit).await;
        return Err(fail(
            Status::BadRequest,
            format!(
                "{value} is not in the device's MultiLanguage list: {}",
                available.join(", ")
            ),
        ));
    }
    let backup = save_backup("language", &json!({ "General.Location.Language": before }))?;
    within(
        "General.Location.Language write",
        limit,
        connection.config_set("General.Location.Language", json!(value)),
    )
    .await?;
    let (current, available) = read_language(&mut connection, limit).await?;
    let _ = connection.logout_within(limit).await;
    if current.as_deref() != Some(value.as_str()) {
        return Err(fail(
            Status::BadGateway,
            format!(
                "the device read back {} after writing {value}; the original is saved in {backup}",
                current.unwrap_or_default()
            ),
        ));
    }
    Ok(Json(LanguageBody {
        ok: true,
        current,
        available,
        before,
        backup: Some(backup),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[derive(Serialize)]
pub struct ActionBody {
    pub ok: bool,
    pub action: &'static str,
    pub detail: String,
    pub elapsed_ms: u64,
}

#[post("/device/system/reboot?<device>&<confirm>")]
#[allow(clippy::too_many_arguments)]
async fn reboot(
    device: Option<String>,
    confirm: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<ActionBody>, Failure> {
    if confirm.as_deref() != Some("reboot") {
        return Err(fail(
            Status::BadRequest,
            "pass confirm=reboot to restart the camera",
        ));
    }
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let ack = within("OPMachine Reboot", limit, connection.reboot()).await?;
    if matches!(ack, RebootAck::Answered(_)) {
        tokio::time::sleep(REBOOT_HOLD.min(limit)).await;
        let _ = connection.logout_within(limit).await;
    }
    Ok(Json(ActionBody {
        ok: true,
        action: "reboot",
        detail: match ack {
            RebootAck::Answered(ret) => format!("the camera accepted the restart (Ret {ret})"),
            RebootAck::LinkClosed => {
                "the camera closed the link right after the restart request".to_string()
            }
        },
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[post("/device/system/defaults?<device>&<confirm>")]
#[allow(clippy::too_many_arguments)]
async fn defaults(
    device: Option<String>,
    confirm: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<ActionBody>, Failure> {
    if confirm.as_deref() != Some(RESET_WORD) {
        return Err(fail(
            Status::BadRequest,
            format!("type {RESET_WORD} to restore factory defaults"),
        ));
    }
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let ret = within("OPDefaultConfig", limit, connection.restore_defaults()).await?;
    let _ = connection.logout_within(limit).await;
    Ok(Json(ActionBody {
        ok: true,
        action: "defaults",
        detail: format!(
            "the camera restored its factory defaults (Ret {ret}); restart it to apply them"
        ),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[derive(Serialize)]
pub struct FirmwareBody {
    pub ok: bool,
    pub firmware: Option<String>,
    pub build_date: Option<String>,
    pub dev_id: Option<String>,
    pub device_check: Option<Value>,
    pub device_check_error: Option<String>,
    pub server_check: Option<ServerCheck>,
    pub server_check_error: Option<String>,
    pub elapsed_ms: u64,
}

fn dev_type(info: &Value) -> String {
    info.get("DeviceType")
        .and_then(Value::as_u64)
        .and_then(|index| upgrade::DEVICE_TYPES.get(index as usize))
        .unwrap_or(&"IPC")
        .to_string()
}

#[get("/device/system/firmware?<device>")]
async fn firmware_check(
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<FirmwareBody>, Failure> {
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let info = within("SystemInfo", limit, connection.system_info()).await?;
    let abilities = within("SystemFunction", limit, connection.abilities()).await?;
    let (device_check, device_check_error) = if gates(&abilities).cloud_upgrade {
        match within(
            "OPVersionReq",
            limit,
            connection.version_request(UPGRADE_LANGUAGE),
        )
        .await
        {
            Ok(reply) => (Some(reply), None),
            Err((_, body)) => (None, Some(body.message.clone())),
        }
    } else {
        (
            None,
            Some("the device does not report OtherFunction/SupportCfgCloudupgrade".to_string()),
        )
    };
    let _ = connection.logout_within(limit).await;
    let firmware = str_field(&info, "SoftWareVersion");
    let build_date =
        str_field(&info, "BuildTime").and_then(|time| system::firmware_build_date(&time));
    let dev_id = firmware.as_deref().and_then(system::firmware_dev_id);
    let serial = str_field(&info, "SerialNo")
        .or(device.clone())
        .unwrap_or_default();
    let (server_check, server_check_error) = match (&dev_id, &build_date) {
        (Some(dev_id), Some(build_date)) => {
            let query = UpgradeQuery {
                dev_id: dev_id.clone(),
                dev_type: dev_type(&info),
                cur_version: build_date.clone(),
                language: UPGRADE_LANGUAGE.to_string(),
                serial,
                pid: str_field(&info, "Pid"),
            };
            match UpgradeClient::from_env(limit).check(&query).await {
                Ok(check) => (Some(check), None),
                Err(error) => (None, Some(error)),
            }
        }
        _ => (
            None,
            Some("SystemInfo has no SoftWareVersion with six parts or no BuildTime".to_string()),
        ),
    };
    Ok(Json(FirmwareBody {
        ok: true,
        firmware,
        build_date,
        dev_id,
        device_check,
        device_check_error,
        server_check,
        server_check_error,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[derive(Deserialize)]
pub struct StartUpgrade {
    confirm: String,
    file_name: String,
    version_date: String,
}

#[post("/device/system/firmware/start?<device>", data = "<body>")]
#[allow(clippy::too_many_arguments)]
async fn firmware_start(
    device: Option<String>,
    body: Json<StartUpgrade>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<ActionBody>, Failure> {
    if body.confirm != UPGRADE_WORD {
        return Err(fail(
            Status::BadRequest,
            format!("type {UPGRADE_WORD} to start the upgrade"),
        ));
    }
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let info = within("SystemInfo", limit, connection.system_info()).await?;
    let dev_id = str_field(&info, "SoftWareVersion")
        .as_deref()
        .and_then(system::firmware_dev_id)
        .ok_or_else(|| {
            fail(
                Status::BadGateway,
                "SystemInfo has no SoftWareVersion with six parts",
            )
        })?;
    let serial = str_field(&info, "SerialNo")
        .or(device.clone())
        .unwrap_or_default();
    let ret = within(
        "OPStartUpgradeReq",
        limit,
        connection.start_cloud_upgrade(&serial, &dev_id, &body.file_name, &body.version_date),
    )
    .await?;
    let _ = connection.logout_within(limit).await;
    Ok(Json(ActionBody {
        ok: true,
        action: "upgrade",
        detail: format!(
            "the camera accepted the cloud upgrade to {} (Ret {ret})",
            body.file_name
        ),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[post("/device/system/firmware/file?<device>&<confirm>", data = "<image>")]
#[allow(clippy::too_many_arguments)]
async fn firmware_file(
    device: Option<String>,
    confirm: Option<String>,
    image: Data<'_>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<ActionBody>, Failure> {
    if confirm.as_deref() != Some(UPGRADE_WORD) {
        return Err(fail(
            Status::BadRequest,
            format!("type {UPGRADE_WORD} to upload firmware"),
        ));
    }
    let bytes = image
        .open(MAX_FIRMWARE.bytes())
        .into_bytes()
        .await
        .map_err(|error| {
            fail(
                Status::BadRequest,
                format!("could not read the upload: {error}"),
            )
        })?;
    if !bytes.is_complete() {
        return Err(fail(
            Status::PayloadTooLarge,
            "the firmware file is larger than 64 MiB",
        ));
    }
    if bytes.is_empty() {
        return Err(fail(Status::BadRequest, "the firmware file is empty"));
    }
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let ret = within(
        "OPSystemUpgrade",
        limit * 6,
        connection.upgrade_from_file(&bytes),
    )
    .await?;
    let _ = connection.logout_within(limit).await;
    Ok(Json(ActionBody {
        ok: true,
        action: "upgrade-file",
        detail: format!(
            "the camera took {} bytes of firmware (Ret {ret})",
            bytes.len()
        ),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[derive(Serialize)]
pub struct NetworkBody {
    pub ok: bool,
    pub pings: Vec<PingReport>,
    pub ping_errors: Vec<String>,
    pub cloud_status: Option<Value>,
    pub cloud_status_error: Option<String>,
    pub nat: Option<Value>,
    pub nat_error: Option<String>,
    pub wifi: Option<Value>,
    pub wifi_error: Option<String>,
    pub elapsed_ms: u64,
}

fn split<T>(result: Result<T, Failure>) -> (Option<T>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err((_, body)) => (None, Some(body.message.clone())),
    }
}

#[get("/device/system/network?<device>")]
async fn network_check(
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<NetworkBody>, Failure> {
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let abilities = within("SystemFunction", limit, connection.abilities()).await?;
    let common = within(
        "NetWork.NetCommon",
        limit,
        connection.config_get("NetWork.NetCommon"),
    )
    .await
    .ok();
    let gateway = common
        .as_ref()
        .and_then(|reply| reply.get("NetWork.NetCommon"))
        .and_then(|block| block.get("GateWay"))
        .and_then(Value::as_str)
        .and_then(system::hex_ipv4)
        .filter(|address| address != "0.0.0.0");
    let mut hosts: Vec<String> = PING_HOSTS.iter().map(|host| host.to_string()).collect();
    hosts.extend(gateway);
    let mut pings = Vec::new();
    let mut ping_errors = Vec::new();
    for host in &hosts {
        match within("Ping", limit, connection.ping(host, 3, 2)).await {
            Ok(report) => pings.push(report),
            Err((_, body)) => ping_errors.push(format!("{host}: {}", body.message)),
        }
    }
    let (cloud_status, cloud_status_error) = split(
        within(
            "XmCloudStatus",
            limit,
            connection.system_entry("XmCloudStatus"),
        )
        .await,
    );
    let (nat, nat_error) = split(
        within(
            "Status.NatInfo",
            limit,
            connection.config_get("Status.NatInfo"),
        )
        .await
        .map(|reply| reply.get("Status.NatInfo").cloned().unwrap_or(Value::Null)),
    );
    let (wifi, wifi_error) = if gates(&abilities).wifi_signal {
        split(
            within(
                "WifiRouteInfo",
                limit,
                connection.system_entry("WifiRouteInfo"),
            )
            .await,
        )
    } else {
        (
            None,
            Some("the device does not report NetServerFunction/WifiRouteSignalLevel".to_string()),
        )
    };
    let _ = connection.logout_within(limit).await;
    Ok(Json(NetworkBody {
        ok: true,
        pings,
        ping_errors,
        cloud_status,
        cloud_status_error,
        nat,
        nat_error,
        wifi,
        wifi_error,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[derive(Serialize)]
pub struct UsersBody {
    pub ok: bool,
    pub users: Vec<DeviceUser>,
    pub recovery: Option<Value>,
    pub recovery_error: Option<String>,
    pub elapsed_ms: u64,
}

#[get("/device/system/users?<device>")]
async fn users_read(
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<UsersBody>, Failure> {
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let users = within("GetAllUser", limit, connection.all_users()).await?;
    let (recovery, recovery_error) = split(
        within(
            "GetSafetyAbility",
            limit,
            connection.safety("GetSafetyAbility", None),
        )
        .await,
    );
    let _ = connection.logout_within(limit).await;
    Ok(Json(UsersBody {
        ok: true,
        users,
        recovery,
        recovery_error,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[derive(Deserialize)]
pub struct PasswordChange {
    user: String,
    old: String,
    new: String,
}

#[post("/device/system/users/password?<device>", data = "<body>")]
#[allow(clippy::too_many_arguments)]
async fn users_password(
    device: Option<String>,
    body: Json<PasswordChange>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<ActionBody>, Failure> {
    if body.user.trim().is_empty() || body.new.is_empty() {
        return Err(fail(
            Status::BadRequest,
            "user and new password are required",
        ));
    }
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let ret = within(
        "ModifyPassword",
        limit,
        connection.modify_password(body.user.trim(), &body.old, &body.new),
    )
    .await?;
    let _ = connection.logout_within(limit).await;
    Ok(Json(ActionBody {
        ok: true,
        action: "password",
        detail: format!(
            "the camera changed the password of {} (Ret {ret})",
            body.user.trim()
        ),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[derive(Deserialize)]
pub struct RenameUser {
    user: String,
    name: String,
}

#[post("/device/system/users/rename?<device>", data = "<body>")]
#[allow(clippy::too_many_arguments)]
async fn users_rename(
    device: Option<String>,
    body: Json<RenameUser>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<ActionBody>, Failure> {
    let name = body.name.trim();
    if body.user.trim().is_empty() || name.is_empty() {
        return Err(fail(Status::BadRequest, "user and name are required"));
    }
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let ret = within(
        "ModifyUser",
        limit,
        connection.rename_user(body.user.trim(), name),
    )
    .await?;
    let _ = connection.logout_within(limit).await;
    Ok(Json(ActionBody {
        ok: true,
        action: "rename",
        detail: format!(
            "the camera renamed {} to {name} (Ret {ret})",
            body.user.trim()
        ),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[derive(Deserialize)]
pub struct Recovery {
    step: String,
    #[serde(default)]
    answers: Vec<String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Serialize)]
pub struct RecoveryBody {
    pub ok: bool,
    pub step: String,
    pub reply: Value,
    pub elapsed_ms: u64,
}

#[post("/device/system/users/recover?<device>", data = "<body>")]
#[allow(clippy::too_many_arguments)]
async fn users_recover(
    device: Option<String>,
    body: Json<Recovery>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<RecoveryBody>, Failure> {
    let (name, inner) = match body.step.as_str() {
        "questions" => ("GetSafetyQuestion", None),
        "answer" => {
            if body.answers.is_empty() {
                return Err(fail(Status::BadRequest, "answers are required"));
            }
            (
                "CheckSafetyAnswer",
                Some(system::safety_answer_value(&body.answers)),
            )
        }
        "password" => {
            let password = body
                .password
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| fail(Status::BadRequest, "password is required"))?;
            let user = body
                .user
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("admin");
            (
                "SetNewPassword",
                Some(system::new_password_value(user, password)),
            )
        }
        other => {
            return Err(fail(
                Status::BadRequest,
                format!("unknown step {other}; use questions, answer or password"),
            ))
        }
    };
    let started = Instant::now();
    let limit = window();
    let link = Link {
        device: device.as_deref(),
        cfg,
        cloud,
        modes,
        store,
    };
    let mut connection = link.connect(cookies, limit).await?;
    let reply = within(name, limit, connection.safety(name, inner)).await?;
    let _ = connection.logout_within(limit).await;
    Ok(Json(RecoveryBody {
        ok: true,
        step: body.step.clone(),
        reply,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}
