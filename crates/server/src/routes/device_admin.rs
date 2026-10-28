use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use proview_core::cloud::attributes::{AttributeClient, AttributeStore, DeviceAttributes};
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
    let share_base = std::env::var("PROVIEW_SHARE_BASE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| proview_core::cloud::attributes::SHARE_BASE.to_string());
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
    if returned.eq_ignore_ascii_case(wanted) {
        return true;
    }
    returned.len() == wanted.len()
        && returned.contains('*')
        && returned
            .chars()
            .zip(wanted.chars())
            .all(|(masked, real)| masked == '*' || masked.eq_ignore_ascii_case(&real))
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
    let target = match (clean(&body.accept_id), clean(&body.account)) {
        (Some(id), _) => id,
        (None, Some(account)) => {
            let hits = admin
                .admin
                .search_user(&signed.token, &account)
                .await
                .map_err(cloud_failure)?;
            hits.iter()
                .find(|hit| account_matches(&hit.account, &account))
                .or_else(|| hits.first())
                .map(|hit| hit.id.clone())
                .ok_or_else(|| {
                    fail(
                        Status::NotFound,
                        format!("no account on the cloud matches {account}"),
                    )
                })?
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
        return admin
            .share
            .share_accept_device(&signed.token, &signed.account, &body.password, device)
            .await
            .map(Json)
            .map_err(cloud_failure);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_masked_search_hit_still_matches_the_account_that_asked_for_it() {
        assert!(account_matches(
            "zhafron@example.com",
            "Zhafron@Example.com"
        ));
        assert!(account_matches(
            "zh*****@example.com",
            "zhafron@example.com"
        ));
        assert!(!account_matches(
            "zh*****@example.com",
            "someone@example.com"
        ));
        assert!(!account_matches(
            "zhafron@example.com",
            "zhafron@example.org"
        ));
        assert!(!account_matches("zh*", "zhafron@example.com"));
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
