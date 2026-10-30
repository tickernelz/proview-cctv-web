use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use proview_core::cloud::attributes::{
    dev_info_plain, permission_list, share_powers, share_qr_link, AttributeClient, AttributeStore,
    DeviceAttributes, ShareEntry, ShareLogin, ShareQrInfo, SHARE_PERMISSIONS, SHARE_QR_LIFETIME_MS,
};
use proview_core::cloud::bind::encode_qr_blob;
use proview_core::cloud::envelope::{EnvelopeClient, Identity};
use proview_core::cloud::{CloudError, CloudSession};
use proview_core::config::Config;
use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};

use super::{fail, session_id, Failure};
use crate::settings::SettingsStore;

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
        share_permissions,
        share_permitted,
        shares_of_device,
        shares_received,
        share_set_permissions,
        share_revoke,
        share_answer,
        share_qr,
    ]
}

const DEFAULT_SHARE_WINDOW_MS: u64 = 12_000;

fn share_window() -> std::time::Duration {
    let millis = std::env::var("PROVIEW_SHARE_WINDOW_MS")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .filter(|value: &u64| *value > 0)
        .unwrap_or(DEFAULT_SHARE_WINDOW_MS);
    std::time::Duration::from_millis(millis)
}

async fn within<T>(
    stage: &str,
    work: impl std::future::Future<Output = Result<T, CloudError>>,
) -> Result<T, Failure> {
    let window = share_window();
    match tokio::time::timeout(window, work).await {
        Ok(result) => result.map_err(cloud_failure),
        Err(_) => Err(fail(
            Status::GatewayTimeout,
            format!(
                "the share service did not answer the {stage} request within {} ms",
                window.as_millis()
            ),
        )),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

fn blob_offset() -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or_default();
    nanos % 699 + 1
}

pub fn share_credentials(cfg: &Config, uuid: &str, configured: &str) -> (String, String) {
    let configured = configured.trim();
    if !configured.is_empty() && configured.eq_ignore_ascii_case(uuid.trim()) {
        let user = if cfg.device_user.trim().is_empty() {
            "admin".to_string()
        } else {
            cfg.device_user.trim().to_string()
        };
        return (user, cfg.device_pass.clone());
    }
    ("admin".to_string(), String::new())
}

fn picked_permissions(wanted: &[String]) -> Result<String, Failure> {
    permission_list(wanted).map_err(|error| fail(Status::BadRequest, error.to_string()))
}

fn share_id_of(raw: &str) -> Result<String, Failure> {
    let id = raw.trim();
    let shaped = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if !shaped {
        return Err(fail(Status::BadRequest, "that is not a share id"));
    }
    Ok(id.to_string())
}

fn serial_of(raw: &str) -> Result<String, Failure> {
    let serial = raw.trim();
    if !proview_core::cloud::bind::serial_ok(serial) {
        return Err(fail(Status::BadRequest, "that is not a device serial"));
    }
    Ok(serial.to_string())
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

struct CloudClients {
    admin: AttributeClient,
    share: AttributeClient,
    cloud: EnvelopeClient,
}

fn clients(cfg: &Config) -> Arc<CloudClients> {
    static POOL: OnceLock<Mutex<HashMap<String, Arc<CloudClients>>>> = OnceLock::new();
    let pool = POOL.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = pool.lock().unwrap_or_else(|error| error.into_inner());
    let share_base = cfg.share_base.clone();
    guard
        .entry(format!("{}|{}", cfg.cloud_base, share_base))
        .or_insert_with(|| {
            let identity = Identity::resolve();
            Arc::new(CloudClients {
                admin: AttributeClient::with_base(identity.clone(), cfg.cloud_base.clone()),
                share: AttributeClient::with_base(identity.clone(), share_base.clone()),
                cloud: EnvelopeClient::with_base(identity, cfg.cloud_base.clone()),
            })
        })
        .clone()
}

fn store_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn load_store() -> Result<AttributeStore, Failure> {
    AttributeStore::load(&store_path()).map_err(|error| {
        fail(
            Status::InternalServerError,
            format!("the group store cannot be read: {error}"),
        )
    })
}

fn save_store(saved: &AttributeStore) -> Result<(), Failure> {
    saved.save(&store_path()).map_err(|error| {
        fail(
            Status::InternalServerError,
            format!("could not persist: {error}"),
        )
    })
}

fn guard_session(store: &State<Store>, cookies: &CookieJar<'_>) -> Result<(), Failure> {
    let id = session_id(cookies)?;
    store
        .get(&id)
        .map(|_| ())
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))
}

fn persist_device_password(settings: &State<SettingsStore>, password: &str) -> Result<(), Failure> {
    let mut incoming = BTreeMap::new();
    incoming.insert("PROVIEW_DEVICE_PASS".to_string(), password.to_string());
    settings.update(incoming).map(|_| ()).map_err(|error| {
        fail(
            Status::InternalServerError,
            format!("the device accepted the new password but it could not be stored: {error}"),
        )
    })
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
    let cloud = clients(cfg);
    let session = cloud_session(&signed);
    let devices = cloud
        .cloud
        .device_list(&session.access_token)
        .await
        .map_err(cloud_failure)?;
    let saved = load_store()?;
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
fn groups(store: &State<Store>, cookies: &CookieJar<'_>) -> Result<Json<GroupsBody>, Failure> {
    guard_session(store, cookies)?;
    let saved = load_store()?;
    Ok(Json(GroupsBody {
        ok: true,
        groups: saved.groups(),
    }))
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
    let admin = clients(cfg);
    admin
        .admin
        .rename(&signed.token, uuid, &signed.account, body.nickname.trim())
        .await
        .map_err(cloud_failure)?;
    Ok(Json(StatusBody {
        ok: true,
        message: format!("renamed {uuid} to {}", body.nickname.trim()),
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
    guard_session(store, cookies)?;
    let uuid = body.uuid.trim();
    if uuid.is_empty() {
        return Err(fail(Status::BadRequest, "uuid is required"));
    }
    let _writing = store_lock();
    let mut saved = load_store()?;
    let mut organisation = saved.get(uuid);
    organisation.group = body.group.trim().to_string();
    saved.set(uuid, organisation);
    save_store(&saved)?;
    Ok(Json(StatusBody {
        ok: true,
        message: format!("group of {uuid} is now {}", body.group.trim()),
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
    guard_session(store, cookies)?;
    let uuid = body.uuid.trim();
    if uuid.is_empty() {
        return Err(fail(Status::BadRequest, "uuid is required"));
    }
    let _writing = store_lock();
    let mut saved = load_store()?;
    let mut organisation = saved.get(uuid);
    organisation.favourite = body.favourite;
    saved.set(uuid, organisation);
    save_store(&saved)?;
    Ok(Json(StatusBody {
        ok: true,
        message: format!("favourite of {uuid} is now {}", body.favourite),
    }))
}

#[derive(Deserialize)]
pub struct ShareBody {
    uuids: Vec<String>,
    #[serde(default, rename = "acceptId", alias = "accept_id")]
    accept_id: Option<String>,
    #[serde(default)]
    account: Option<String>,
}

fn clean(raw: &Option<String>) -> Option<String> {
    raw.as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn account_matches(returned: &str, wanted: &str) -> bool {
    let returned: Vec<char> = returned.trim().to_lowercase().chars().collect();
    let wanted: Vec<char> = wanted.trim().to_lowercase().chars().collect();
    if returned == wanted {
        return true;
    }
    let Some(first) = returned.iter().position(|character| *character == '*') else {
        return false;
    };
    let last = returned
        .iter()
        .rposition(|character| *character == '*')
        .unwrap_or(first);
    let prefix = &returned[..first];
    let suffix = &returned[last + 1..];
    wanted.len() >= prefix.len() + suffix.len()
        && wanted.starts_with(prefix)
        && wanted.ends_with(suffix)
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
    let admin = clients(cfg);
    let (target, resolved) = match (clean(&body.accept_id), clean(&body.account)) {
        (Some(id), _) => (id, None),
        (None, Some(account)) => {
            let (id, masked) = resolve_target(&admin.admin, &signed.token, &account).await?;
            (id, Some(masked))
        }
        (None, None) => {
            return Err(fail(
                Status::BadRequest,
                "the receiving account or its account id is required",
            ))
        }
    };
    let reply = admin
        .admin
        .share_out(&signed.token, &body.uuids, &target)
        .await
        .map_err(cloud_failure)?;
    let mut payload = reply;
    if let (Some(masked), serde_json::Value::Object(fields)) = (resolved, &mut payload) {
        fields.insert("resolved".to_string(), serde_json::Value::String(masked));
    }
    Ok(Json(payload))
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
    let admin = clients(cfg);
    admin
        .admin
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
    let admin = clients(cfg);
    admin
        .admin
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

#[derive(Deserialize)]
pub struct AcceptBody {
    #[serde(default)]
    key: String,
    #[serde(default, rename = "devId", alias = "dev_id")]
    dev_id: String,
    #[serde(default)]
    password: String,
}

#[post("/cloud/device/share/accept", data = "<body>")]
async fn share_accept(
    body: Json<AcceptBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<serde_json::Value>, Failure> {
    let signed = signed(store, cookies)?;
    let admin = clients(cfg);
    let device = body.dev_id.trim();
    if !device.is_empty() {
        if body.password.is_empty() {
            return Err(fail(
                Status::BadRequest,
                "the receiver must send its account password with the device id",
            ));
        }
        let reply = admin
            .share
            .share_accept_device(&signed.token, &signed.account, &body.password, device)
            .await
            .map_err(cloud_failure)?;
        let devices = admin
            .cloud
            .device_list(&signed.token)
            .await
            .map_err(cloud_failure)?;
        if devices.iter().any(|entry| entry.uuid == device) {
            return Ok(Json(serde_json::json!({
                "ok": true,
                "materialised": true,
                "vendor": reply,
            })));
        }
        return Err(fail(
            Status::Conflict,
            format!("the share service answered 2000 for {device}, but the camera is not in this account's device list, so the acceptance did not take effect"),
        ));
    }
    let key = body.key.trim();
    if key.is_empty() {
        return Err(fail(
            Status::BadRequest,
            "an invitation key or a device id is required",
        ));
    }
    admin
        .admin
        .share_accept(&signed.token, key)
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
    let admin = clients(cfg);
    admin
        .admin
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
    settings: &State<SettingsStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StatusBody>, Failure> {
    let signed = signed(store, cookies)?;
    let uuid = body.uuid.trim();
    if uuid.is_empty() {
        return Err(fail(Status::BadRequest, "uuid is required"));
    }
    let next = body.new_password.trim();
    if next.is_empty() {
        return Err(fail(Status::BadRequest, "the new password is empty"));
    }
    let exported = std::env::var("PROVIEW_DEVICE_PASS")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let admin = clients(cfg);
    admin
        .admin
        .change_login_password(&signed.token, uuid, &body.password, next)
        .await
        .map_err(cloud_failure)?;
    persist_device_password(settings, next)?;
    let source = match exported {
        Some(previous) if previous != next => {
            "PROVIEW_DEVICE_PASS was already exported (or comes from the desktop keychain), and that source still holds the old password: change it too, a restart would bring it back"
        }
        _ => "the new password is in settings.json; restart the app so every camera route picks it up",
    };
    Ok(Json(StatusBody {
        ok: true,
        message: format!("the cloud changed the device password of {uuid}; {source}"),
    }))
}

#[derive(Serialize)]
pub struct PermissionOption {
    pub id: &'static str,
    pub label: &'static str,
    pub default: bool,
}

#[get("/cloud/device/share/permissions")]
fn share_permissions(
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<Vec<PermissionOption>>, Failure> {
    guard_session(store, cookies)?;
    Ok(Json(
        SHARE_PERMISSIONS
            .iter()
            .map(|(id, label, default)| PermissionOption {
                id,
                label,
                default: *default,
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct PermittedBody {
    uuid: String,
    #[serde(default, rename = "acceptId", alias = "accept_id")]
    accept_id: Option<String>,
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    password: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermittedReply {
    pub ok: bool,
    pub uuid: String,
    pub permissions: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
    pub vendor: serde_json::Value,
}

#[post("/cloud/device/share/permitted", data = "<body>")]
async fn share_permitted(
    body: Json<PermittedBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<PermittedReply>, Failure> {
    let signed = signed(store, cookies)?;
    let uuid = serial_of(&body.uuid)?;
    let permissions = picked_permissions(&body.permissions)?;
    let admin = clients(cfg);
    let (target, resolved) = match (clean(&body.accept_id), clean(&body.account)) {
        (Some(id), _) => (id, None),
        (None, Some(account)) => {
            let hits = within(
                "account search",
                admin.admin.search_user(&signed.token, &account),
            )
            .await?;
            let matching: Vec<_> = hits
                .iter()
                .filter(|hit| account_matches(&hit.account, &account))
                .collect();
            match matching.as_slice() {
                [hit] => (hit.id.clone(), Some(hit.account.clone())),
                [] => {
                    return Err(fail(
                        Status::NotFound,
                        format!("no account on the cloud matches {account}"),
                    ))
                }
                many => {
                    return Err(fail(
                        Status::Conflict,
                        format!(
                            "{account} matches {} accounts on the cloud; send the receiving account id instead",
                            many.len()
                        ),
                    ))
                }
            }
        }
        (None, None) => {
            return Err(fail(
                Status::BadRequest,
                "the receiving account or its account id is required",
            ))
        }
    };
    let (user, password) =
        share_credentials(cfg, &uuid, &proview_core::device::configured_serial());
    let plain = dev_info_plain(&uuid, &user, &password, 0, now_secs());
    let powers = share_powers(&encode_qr_blob(&plain, blob_offset()));
    let vendor = within(
        "share",
        admin.admin.share_with_permissions(
            &signed.token,
            &ShareLogin::new(&signed.account, &body.password),
            &uuid,
            &target,
            &powers,
            &permissions,
        ),
    )
    .await?;
    Ok(Json(PermittedReply {
        ok: true,
        uuid,
        permissions,
        resolved,
        vendor,
    }))
}

#[get("/cloud/device/shares?<uuid>")]
async fn shares_of_device(
    uuid: String,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<Vec<ShareEntry>>, Failure> {
    let signed = signed(store, cookies)?;
    let uuid = serial_of(&uuid)?;
    let admin = clients(cfg);
    within(
        "share list",
        admin
            .admin
            .shares_of_device(&signed.token, &ShareLogin::default(), &uuid),
    )
    .await
    .map(Json)
}

#[get("/cloud/device/shares/received")]
async fn shares_received(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<Vec<ShareEntry>>, Failure> {
    let signed = signed(store, cookies)?;
    let admin = clients(cfg);
    within(
        "received shares",
        admin
            .admin
            .shares_received(&signed.token, &ShareLogin::default()),
    )
    .await
    .map(Json)
}

#[derive(Deserialize)]
pub struct SharePermissionBody {
    #[serde(rename = "shareId", alias = "share_id")]
    share_id: String,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    password: String,
}

#[post("/cloud/device/shares/permissions", data = "<body>")]
async fn share_set_permissions(
    body: Json<SharePermissionBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<serde_json::Value>, Failure> {
    let signed = signed(store, cookies)?;
    let id = share_id_of(&body.share_id)?;
    let permissions = picked_permissions(&body.permissions)?;
    let admin = clients(cfg);
    within(
        "permission change",
        admin.admin.set_share_permissions(
            &signed.token,
            &ShareLogin::new(&signed.account, &body.password),
            &id,
            &permissions,
        ),
    )
    .await
    .map(Json)
}

#[derive(Deserialize)]
pub struct ShareIdBody {
    #[serde(rename = "shareId", alias = "share_id")]
    share_id: String,
    #[serde(default)]
    accept: Option<bool>,
    #[serde(default)]
    password: String,
}

#[post("/cloud/device/shares/revoke", data = "<body>")]
async fn share_revoke(
    body: Json<ShareIdBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<serde_json::Value>, Failure> {
    let signed = signed(store, cookies)?;
    let id = share_id_of(&body.share_id)?;
    let admin = clients(cfg);
    let login = ShareLogin::new(&signed.account, &body.password);
    within(
        "revoke",
        admin.admin.revoke_share(&signed.token, &login, &id),
    )
    .await
    .map(Json)
}

#[post("/cloud/device/shares/answer", data = "<body>")]
async fn share_answer(
    body: Json<ShareIdBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<serde_json::Value>, Failure> {
    let signed = signed(store, cookies)?;
    let id = share_id_of(&body.share_id)?;
    let accept = body.accept.ok_or_else(|| {
        fail(
            Status::BadRequest,
            "say whether the share is accepted or refused",
        )
    })?;
    let admin = clients(cfg);
    let login = ShareLogin::new(&signed.account, &body.password);
    if accept {
        within(
            "accept",
            admin
                .admin
                .accept_permission_share(&signed.token, &login, &id),
        )
        .await
        .map(Json)
    } else {
        within(
            "refuse",
            admin.admin.refuse_share(&signed.token, &login, &id),
        )
        .await
        .map(Json)
    }
}

#[derive(Deserialize)]
pub struct ShareQrBody {
    uuid: String,
    #[serde(default)]
    permissions: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareQrReply {
    pub link: String,
    pub svg: String,
    pub expires_at: i64,
    pub lifetime_secs: u64,
    pub permissions: String,
}

pub fn share_qr_info(
    uuid: &str,
    user_id: &str,
    credentials: (String, String),
    permissions: &str,
    now: i64,
) -> ShareQrInfo {
    let (login_name, pwd) = credentials;
    ShareQrInfo {
        dev_id: uuid.to_string(),
        dev_type: 0,
        user_id: user_id.to_string(),
        login_name,
        pwd,
        share_times: now,
        permissions: permissions.to_string(),
    }
}

pub fn qr_svg(text: &str) -> Result<String, Failure> {
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(|error| {
        fail(
            Status::InternalServerError,
            format!("the share link does not fit a QR code: {error}"),
        )
    })?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(232, 232)
        .quiet_zone(true)
        .build())
}

#[post("/cloud/device/share/qr", data = "<body>")]
async fn share_qr(
    body: Json<ShareQrBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<ShareQrReply>, Failure> {
    let signed = signed(store, cookies)?;
    let uuid = serial_of(&body.uuid)?;
    let permissions = picked_permissions(&body.permissions)?;
    let user_id = proview_core::cloud::claims(&signed.token)
        .and_then(|claims| claims.account)
        .unwrap_or_default();
    let now = now_secs();
    let credentials = share_credentials(cfg, &uuid, &proview_core::device::configured_serial());
    let info = share_qr_info(&uuid, &user_id, credentials, &permissions, now);
    let plain = serde_json::to_string(&info).map_err(|error| {
        fail(
            Status::InternalServerError,
            format!("the share code cannot be built: {error}"),
        )
    })?;
    let blob = encode_qr_blob(&plain, blob_offset());
    let admin = clients(cfg);
    let (key, expires) = within(
        "share code",
        admin
            .admin
            .save_share_info(&signed.token, &blob, SHARE_QR_LIFETIME_MS),
    )
    .await?;
    let link = share_qr_link(&key);
    Ok(Json(ShareQrReply {
        svg: qr_svg(&link)?,
        link,
        expires_at: if expires > 0 {
            expires
        } else {
            now * 1000 + SHARE_QR_LIFETIME_MS as i64
        },
        lifetime_secs: SHARE_QR_LIFETIME_MS / 1000,
        permissions,
    }))
}

async fn resolve_target(
    admin: &AttributeClient,
    token: &str,
    account: &str,
) -> Result<(String, String), Failure> {
    let hits = admin
        .search_user(token, account)
        .await
        .map_err(cloud_failure)?;
    let matching: Vec<&proview_core::cloud::attributes::UserSearchHit> = hits
        .iter()
        .filter(|hit| account_matches(&hit.account, account))
        .collect();
    match matching.as_slice() {
        [] => Err(fail(
            Status::NotFound,
            format!("no account on the cloud matches {account}"),
        )),
        [hit] => Ok((hit.id.clone(), hit.account.clone())),
        many => Err(fail(
            Status::Conflict,
            format!(
                "{account} matches {} accounts on the cloud; send the receiving account id instead",
                many.len()
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_masked_hit_matches_on_its_prefix_and_suffix_only() {
        assert!(account_matches(
            "zhafron@example.com",
            "Zhafron@Example.com"
        ));
        assert!(account_matches(
            "zh***@example.com",
            "zhexample@example.com"
        ));
        assert!(account_matches(
            "x***@example.invalid",
            "xample@example.invalid"
        ));
        assert!(!account_matches(
            "x***@example.invalid",
            "someone@example.invalid"
        ));
        assert!(!account_matches(
            "x***@example.invalid",
            "xample@example.com"
        ));
        assert!(!account_matches(
            "zhafron@example.com",
            "zhafron@example.org"
        ));
    }

    #[test]
    fn the_share_target_prefers_the_explicit_id_over_the_account() {
        let body = ShareBody {
            uuids: vec!["DEV00000000000000001".to_string()],
            accept_id: Some("  ".to_string()),
            account: Some("other@example.com".to_string()),
        };
        assert_eq!(clean(&body.accept_id), None);
        assert_eq!(clean(&body.account).as_deref(), Some("other@example.com"));
    }
}
