use std::path::PathBuf;

use proview_core::cloud::attributes::{AttributeClient, AttributeStore, DeviceAttributes};
use proview_core::cloud::envelope::{EnvelopeClient, Identity};
use proview_core::cloud::{CloudError, CloudSession};
use proview_core::config::Config;
use proview_core::device::{ControlError, Controls};
use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};

use super::{fail, session_id, Failure};

pub const STORE_FILE: &str = "device-groups.json";

pub fn routes() -> Vec<rocket::route::Route> {
    routes![
        attributes,
        groups,
        rename,
        set_group,
        set_favourite,
        share,
        share_pending,
        share_info,
        share_accept,
        share_cancel,
        change_password,
        change_device_password,
    ]
}

fn store_path() -> PathBuf {
    let dir = std::env::var("PROVIEW_DATA_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| ".".to_string());
    PathBuf::from(dir).join(STORE_FILE)
}

fn cloud_status(error: &CloudError) -> Status {
    match error {
        CloudError::Rejected(_) => Status::BadRequest,
        CloudError::Credentials | CloudError::Expired => Status::Unauthorized,
        CloudError::Unreachable(_) => Status::ServiceUnavailable,
        CloudError::Malformed(_) => Status::BadGateway,
    }
}

fn cloud_failure(error: CloudError) -> Failure {
    fail(cloud_status(&error), error.to_string())
}

fn control_status(error: &ControlError) -> Status {
    match error {
        ControlError::Unsupported { .. } => Status::NotImplemented,
        ControlError::NotConfigured(_) => Status::BadRequest,
        ControlError::Blocked { .. } => Status::Conflict,
        ControlError::Device(_) => Status::BadGateway,
    }
}

struct Signed {
    account: String,
    token: String,
}

fn signed(store: &State<Store>, cookies: &CookieJar<'_>) -> Result<Signed, Failure> {
    let id = session_id(cookies)?;
    let vendor = store
        .get(&id)
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))?;
    let token = vendor.token.ok_or_else(|| {
        fail(
            Status::Conflict,
            "this session is on the local device, not the cloud",
        )
    })?;
    Ok(Signed {
        account: vendor.account,
        token,
    })
}

fn clients(cfg: &Config) -> (AttributeClient, EnvelopeClient) {
    let identity = Identity::resolve();
    (
        AttributeClient::with_base(identity.clone(), cfg.cloud_base.clone()),
        EnvelopeClient::with_base(identity, cfg.cloud_base.clone()),
    )
}

fn cloud_session(signed: &Signed) -> CloudSession {
    CloudSession {
        account: signed.account.clone(),
        access_token: signed.token.clone(),
    }
}

#[derive(Serialize)]
pub struct AttributeBody {
    pub ok: bool,
    pub groups: Vec<String>,
    pub devices: Vec<DeviceAttributes>,
}

#[get("/cloud/device-attributes")]
async fn attributes(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<AttributeBody>, Failure> {
    let signed = signed(store, cookies)?;
    let (_, cloud) = clients(cfg);
    let session = cloud_session(&signed);
    let devices = cloud
        .device_list(&session.access_token)
        .await
        .map_err(cloud_failure)?;
    let saved = AttributeStore::load(&store_path());
    let rows: Vec<DeviceAttributes> = devices
        .iter()
        .map(|device| DeviceAttributes::of(device, &saved.get(&device.uuid)))
        .collect();
    Ok(Json(AttributeBody {
        ok: true,
        groups: saved.groups(),
        devices: rows,
    }))
}

#[derive(Serialize)]
pub struct GroupsBody {
    pub ok: bool,
    pub groups: Vec<String>,
}

#[get("/cloud/device-groups")]
fn groups() -> Json<GroupsBody> {
    Json(GroupsBody {
        ok: true,
        groups: AttributeStore::load(&store_path()).groups(),
    })
}

#[derive(Deserialize)]
pub struct RenameBody {
    uuid: String,
    nickname: String,
}

#[derive(Serialize)]
pub struct StatusBody {
    pub ok: bool,
    pub message: String,
}

#[post("/cloud/device/rename", data = "<body>")]
async fn rename(
    body: Json<RenameBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StatusBody>, Failure> {
    let signed = signed(store, cookies)?;
    let uuid = body.uuid.trim();
    if uuid.is_empty() {
        return Err(fail(Status::BadRequest, "uuid is required"));
    }
    let (admin, _) = clients(cfg);
    admin
        .rename(&signed.token, uuid, &signed.account, body.nickname.trim())
        .await
        .map_err(cloud_failure)?;
    Ok(Json(StatusBody {
        ok: true,
        message: format!("renamed {uuid} to {:?}", body.nickname.trim()),
    }))
}

#[derive(Deserialize)]
pub struct GroupBody {
    uuid: String,
    #[serde(default)]
    group: String,
}

#[post("/cloud/device/group", data = "<body>")]
fn set_group(
    body: Json<GroupBody>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StatusBody>, Failure> {
    let id = session_id(cookies)?;
    store
        .get(&id)
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))?;
    let uuid = body.uuid.trim();
    if uuid.is_empty() {
        return Err(fail(Status::BadRequest, "uuid is required"));
    }
    let path = store_path();
    let mut saved = AttributeStore::load(&path);
    let mut organisation = saved.get(uuid);
    organisation.group = body.group.trim().to_string();
    saved.set(uuid, organisation);
    saved.save(&path).map_err(|error| {
        fail(
            Status::InternalServerError,
            format!("could not persist: {error}"),
        )
    })?;
    Ok(Json(StatusBody {
        ok: true,
        message: format!("group of {uuid} is now {:?}", body.group.trim()),
    }))
}

#[derive(Deserialize)]
pub struct FavouriteBody {
    uuid: String,
    favourite: bool,
}

#[post("/cloud/device/favourite", data = "<body>")]
fn set_favourite(
    body: Json<FavouriteBody>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StatusBody>, Failure> {
    let id = session_id(cookies)?;
    store
        .get(&id)
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))?;
    let uuid = body.uuid.trim();
    if uuid.is_empty() {
        return Err(fail(Status::BadRequest, "uuid is required"));
    }
    let path = store_path();
    let mut saved = AttributeStore::load(&path);
    let mut organisation = saved.get(uuid);
    organisation.favourite = body.favourite;
    saved.set(uuid, organisation);
    saved.save(&path).map_err(|error| {
        fail(
            Status::InternalServerError,
            format!("could not persist: {error}"),
        )
    })?;
    Ok(Json(StatusBody {
        ok: true,
        message: format!("favourite of {uuid} is now {}", body.favourite),
    }))
}

#[derive(Deserialize)]
pub struct ShareBody {
    uuids: Vec<String>,
    #[serde(rename = "acceptId", alias = "accept_id")]
    accept_id: String,
}

#[post("/cloud/device/share", data = "<body>")]
async fn share(
    body: Json<ShareBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<serde_json::Value>, Failure> {
    let signed = signed(store, cookies)?;
    if body.uuids.is_empty() {
        return Err(fail(Status::BadRequest, "at least one uuid is required"));
    }
    let (admin, _) = clients(cfg);
    let reply = admin
        .share_out(&signed.token, &body.uuids, body.accept_id.trim())
        .await
        .map_err(cloud_failure)?;
    Ok(Json(reply))
}

#[get("/cloud/device/share/pending?<uuids>")]
async fn share_pending(
    uuids: String,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<serde_json::Value>, Failure> {
    let signed = signed(store, cookies)?;
    let list: Vec<String> = uuids
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect();
    if list.is_empty() {
        return Err(fail(Status::BadRequest, "at least one uuid is required"));
    }
    let (admin, _) = clients(cfg);
    admin
        .share_pending(&signed.token, &list)
        .await
        .map(Json)
        .map_err(cloud_failure)
}

#[get("/cloud/device/share/info?<key>")]
async fn share_info(
    key: String,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<serde_json::Value>, Failure> {
    let signed = signed(store, cookies)?;
    let (admin, _) = clients(cfg);
    admin
        .share_info(&signed.token, key.trim())
        .await
        .map(Json)
        .map_err(cloud_failure)
}

#[derive(Deserialize)]
pub struct KeyBody {
    key: String,
    #[serde(default)]
    uuids: Vec<String>,
}

#[post("/cloud/device/share/accept", data = "<body>")]
async fn share_accept(
    body: Json<KeyBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<serde_json::Value>, Failure> {
    let signed = signed(store, cookies)?;
    let (admin, _) = clients(cfg);
    admin
        .share_accept(&signed.token, body.key.trim())
        .await
        .map(Json)
        .map_err(cloud_failure)
}

#[post("/cloud/device/share/cancel", data = "<body>")]
async fn share_cancel(
    body: Json<KeyBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<serde_json::Value>, Failure> {
    let signed = signed(store, cookies)?;
    let (admin, _) = clients(cfg);
    admin
        .share_cancel(&signed.token, body.key.trim(), &body.uuids)
        .await
        .map(Json)
        .map_err(cloud_failure)
}

#[derive(Deserialize)]
pub struct CloudPasswordBody {
    uuid: String,
    password: String,
    #[serde(rename = "newPassword", alias = "new_password")]
    new_password: String,
}

#[post("/cloud/device/password", data = "<body>")]
async fn change_password(
    body: Json<CloudPasswordBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StatusBody>, Failure> {
    let signed = signed(store, cookies)?;
    let uuid = body.uuid.trim();
    if uuid.is_empty() {
        return Err(fail(Status::BadRequest, "uuid is required"));
    }
    let (admin, _) = clients(cfg);
    admin
        .change_login_password(&signed.token, uuid, &body.password, &body.new_password)
        .await
        .map_err(cloud_failure)?;
    Ok(Json(StatusBody {
        ok: true,
        message: format!("the cloud changed the device password of {uuid}"),
    }))
}

#[derive(Deserialize)]
pub struct DevicePasswordBody {
    #[serde(rename = "newPassword", alias = "new_password")]
    new_password: String,
}

#[post("/device/password", data = "<body>")]
async fn change_device_password(
    body: Json<DevicePasswordBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StatusBody>, Failure> {
    let id = session_id(cookies)?;
    store
        .get(&id)
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))?;
    if body.new_password.trim().is_empty() {
        return Err(fail(Status::BadRequest, "the new password is empty"));
    }
    if cfg.device_host.is_none() {
        return Err(fail(
            Status::BadRequest,
            ControlError::NotConfigured("PROVIEW_DEVICE_HOST").to_string(),
        ));
    }
    let controls = Controls::from_config(cfg);
    let dvrip = controls
        .dvrip()
        .map_err(|error| fail(control_status(&error), error.to_string()))?;
    let mut session = dvrip
        .session()
        .await
        .map_err(|error| fail(control_status(&error), error.to_string()))?;
    let ret = proview_core::cloud::attributes::change_device_password(
        &mut session,
        &cfg.device_user,
        &cfg.device_pass,
        body.new_password.trim(),
    )
    .await
    .map_err(|error| {
        fail(
            Status::BadGateway,
            format!("the device refused the password change: {error}"),
        )
    })?;
    Ok(Json(StatusBody {
        ok: true,
        message: format!("the device changed its login password (ret {ret})"),
    }))
}
