use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use proview_core::cloud::bind::{parse_qr, AddDevice, BindClient, BindStatus, SignOutReply};
use proview_core::cloud::claims;
use proview_core::cloud::{CloudError, Identity};
use proview_core::config::Config;
use proview_core::provider::VendorSession;
use proview_core::session::{new_id, Store};
use rocket::http::{Cookie, CookieJar, SameSite, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};

use crate::routes::{fail, fail_code, session_id, Failure};

const FILE: &str = "account.json";
const COOKIE: &str = "pvw_session";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Remembered {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub saved_at: i64,
}

pub struct AccountStore {
    path: Option<PathBuf>,
    inner: RwLock<Remembered>,
}

impl AccountStore {
    pub fn load(data_dir: Option<&Path>) -> Self {
        let dir = data_dir.map(Path::to_path_buf).or_else(|| {
            std::env::var("PROVIEW_DATA_DIR")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        });
        let path = dir.map(|dir| dir.join(FILE));
        let remembered = path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|raw| serde_json::from_str::<Remembered>(&raw).ok())
            .unwrap_or_default();
        Self {
            path,
            inner: RwLock::new(remembered),
        }
    }

    pub fn from_env() -> Self {
        Self::load(None)
    }

    pub fn snapshot(&self) -> Remembered {
        self.inner.read().expect("account store poisoned").clone()
    }

    fn store(&self, remembered: Remembered) {
        *self.inner.write().expect("account store poisoned") = remembered.clone();
        if let Some(path) = &self.path {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(raw) = serde_json::to_string_pretty(&remembered) {
                if std::fs::write(path, raw).is_ok() {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ =
                            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
                    }
                }
            }
        }
    }

    pub fn remember(&self, email: &str, token: &str, expires_at: Option<i64>) {
        self.store(Remembered {
            email: email.trim().to_string(),
            access_token: token.to_string(),
            expires_at,
            saved_at: now_seconds(),
        });
    }

    pub fn forget_token(&self) {
        let mut current = self.snapshot();
        current.access_token.clear();
        current.expires_at = None;
        current.saved_at = now_seconds();
        self.store(current);
    }
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

fn token_live(remembered: &Remembered) -> bool {
    if remembered.access_token.trim().is_empty() {
        return false;
    }
    match remembered.expires_at {
        Some(expires) => expires - 60 > now_seconds(),
        None => false,
    }
}

fn cloud_failure(error: &CloudError) -> Failure {
    match error {
        CloudError::Credentials => fail_code(
            Status::BadRequest,
            "device_credentials",
            "the camera refused that username and password",
        ),
        CloudError::Expired => fail(Status::Unauthorized, "session expired; sign in again"),
        CloudError::Rejected(message) => {
            fail_code(Status::BadGateway, "cloud_rejected", message.clone())
        }
        CloudError::Unreachable(message) => {
            fail_code(Status::BadGateway, "cloud_unreachable", message.clone())
        }
        CloudError::Malformed(message) => {
            fail_code(Status::BadGateway, "cloud_malformed", message.clone())
        }
    }
}

fn live_session(
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<(String, VendorSession), Failure> {
    let id = session_id(cookies)?;
    let session = store
        .get(&id)
        .ok_or_else(|| fail(Status::Unauthorized, "session expired; sign in again"))?;
    Ok((id, session))
}

#[derive(Serialize)]
pub struct BindReply {
    pub ok: bool,
    pub status: &'static str,
    pub serial: String,
    pub vendor_code: i32,
    pub message: String,
}

#[derive(Deserialize)]
pub struct BindBody {
    pub sn: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub nickname: String,
}

#[derive(Deserialize)]
pub struct QrBody {
    pub payload: String,
    #[serde(default)]
    pub password: Option<String>,
}

fn request_for(body: &BindBody, session: &VendorSession) -> Result<AddDevice, Failure> {
    let serial = body.sn.trim();
    if serial.is_empty()
        || serial.len() > 32
        || !serial.bytes().all(|value| value.is_ascii_alphanumeric())
    {
        return Err(fail_code(
            Status::BadRequest,
            "bad_serial",
            "a device serial is 8 to 32 letters and digits",
        ));
    }
    let token = session.token.as_deref().ok_or_else(|| {
        fail(
            Status::Unauthorized,
            "session carries no cloud token; sign in again",
        )
    })?;
    let account_id = claims(token)
        .and_then(|value| value.account)
        .unwrap_or_default();
    Ok(AddDevice {
        serial: serial.to_string(),
        username: if body.username.trim().is_empty() {
            "admin".to_string()
        } else {
            body.username.trim().to_string()
        },
        password: body.password.clone(),
        account_id,
        nickname: if body.nickname.trim().is_empty() {
            serial.to_string()
        } else {
            body.nickname.trim().to_string()
        },
        kind: 0,
        ip: "0.0.0.0".to_string(),
        port: 34567,
    })
}

async fn bind(
    request: AddDevice,
    session: &VendorSession,
    cfg: &State<Config>,
) -> Result<Json<BindReply>, Failure> {
    let token = session.token.as_deref().ok_or_else(|| {
        fail(
            Status::Unauthorized,
            "session carries no cloud token; sign in again",
        )
    })?;
    let client = BindClient::new(Identity::resolve(), cfg.cloud_base.clone());
    let report = client
        .add_device(token, &request)
        .await
        .map_err(|error| cloud_failure(&error))?;
    Ok(Json(BindReply {
        ok: true,
        status: match report.status {
            BindStatus::Bound => "bound",
            BindStatus::AlreadyBound => "already_bound",
        },
        message: report.message(),
        serial: report.serial,
        vendor_code: report.vendor_code,
    }))
}

#[post("/devices/bind", data = "<body>")]
async fn bind_by_serial(
    body: Json<BindBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<BindReply>, Failure> {
    let (_, session) = live_session(store, cookies)?;
    let request = request_for(&body, &session)?;
    bind(request, &session, cfg).await
}

#[post("/devices/bind/qr", data = "<body>")]
async fn bind_by_qr(
    body: Json<QrBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<BindReply>, Failure> {
    let (_, session) = live_session(store, cookies)?;
    let device = parse_qr(&body.payload)
        .map_err(|error| fail_code(Status::BadRequest, "bad_qr", error.to_string()))?;
    let password = body
        .password
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(device.password);
    let request = request_for(
        &BindBody {
            sn: device.serial,
            username: device.username,
            password,
            nickname: device.name.unwrap_or_default(),
        },
        &session,
    )?;
    bind(request, &session, cfg).await
}

#[post("/devices/unbind", data = "<body>")]
async fn unbind_by_serial(
    body: Json<BindBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<BindReply>, Failure> {
    let (_, session) = live_session(store, cookies)?;
    let serial = body.sn.trim().to_string();
    if serial.is_empty() {
        return Err(fail_code(
            Status::BadRequest,
            "bad_serial",
            "a device serial is required",
        ));
    }
    let token = session.token.as_deref().ok_or_else(|| {
        fail(
            Status::Unauthorized,
            "session carries no cloud token; sign in again",
        )
    })?;
    let client = BindClient::new(Identity::resolve(), cfg.cloud_base.clone());
    let code = client
        .remove_device(token, &serial)
        .await
        .map_err(|error| cloud_failure(&error))?;
    Ok(Json(BindReply {
        ok: true,
        status: "removed",
        message: format!("{serial} is no longer on this account"),
        serial,
        vendor_code: code,
    }))
}

#[derive(Serialize)]
pub struct RememberReply {
    pub ok: bool,
    pub email: String,
    pub restored: bool,
    pub message: String,
}

#[get("/account/remembered")]
fn remembered(
    accounts: &State<AccountStore>,
    store: &State<Store>,
    cfg: &State<Config>,
    cookies: &CookieJar<'_>,
) -> Json<RememberReply> {
    let saved = accounts.snapshot();
    if let Some(id) = cookies
        .get_private(COOKIE)
        .map(|cookie| cookie.value().to_string())
    {
        if store.get(&id).is_some() {
            return Json(RememberReply {
                ok: true,
                email: saved.email.clone(),
                restored: true,
                message: "already signed in".to_string(),
            });
        }
    }
    if !token_live(&saved) {
        let message = if saved.email.is_empty() {
            "no remembered account".to_string()
        } else {
            "the remembered session expired; sign in again".to_string()
        };
        return Json(RememberReply {
            ok: true,
            email: saved.email,
            restored: false,
            message,
        });
    }
    let id = new_id();
    store.put(
        id.clone(),
        VendorSession {
            account: saved.email.clone(),
            token: Some(saved.access_token.clone()),
        },
        Duration::from_secs(cfg.session_ttl_secs),
    );
    cookies.add_private(
        Cookie::build((COOKIE, id))
            .http_only(true)
            .same_site(SameSite::Strict)
            .build(),
    );
    Json(RememberReply {
        ok: true,
        email: saved.email,
        restored: true,
        message: "session restored from the remembered account".to_string(),
    })
}

#[post("/account/remember")]
fn remember(
    accounts: &State<AccountStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<RememberReply>, Failure> {
    let (_, session) = live_session(store, cookies)?;
    let token = session.token.as_deref().ok_or_else(|| {
        fail(
            Status::Unauthorized,
            "session carries no cloud token; sign in again",
        )
    })?;
    let expires_at = claims(token).and_then(|value| value.exp);
    accounts.remember(&session.account, token, expires_at);
    Ok(Json(RememberReply {
        ok: true,
        email: session.account,
        restored: true,
        message: "account remembered".to_string(),
    }))
}

#[derive(Serialize)]
pub struct SignOutBody {
    pub ok: bool,
    pub message: String,
    pub email: String,
    pub vendor: VendorReply,
}

#[derive(Serialize)]
pub struct VendorReply {
    pub http_status: u16,
    pub code: Option<i32>,
    pub message: String,
    pub revoked: bool,
}

#[post("/account/signout")]
async fn sign_out(
    accounts: &State<AccountStore>,
    store: &State<Store>,
    cfg: &State<Config>,
    cookies: &CookieJar<'_>,
) -> Json<SignOutBody> {
    let saved = accounts.snapshot();
    let mut token = saved.access_token;
    if let Some(cookie) = cookies.get_private(COOKIE) {
        if let Some(session) = store.get(cookie.value()) {
            if let Some(current) = session.token {
                token = current;
            }
        }
        store.remove(cookie.value());
        cookies.remove_private(cookie);
    }
    accounts.forget_token();
    let reply: SignOutReply = if token.trim().is_empty() {
        SignOutReply {
            http_status: 0,
            vendor_code: None,
            vendor_message: "no vendor token to revoke".to_string(),
        }
    } else {
        BindClient::new(Identity::resolve(), cfg.cloud_base.clone())
            .vendor_sign_out(&token)
            .await
    };
    Json(SignOutBody {
        ok: true,
        message: "signed out here; the vendor reply is in vendor".to_string(),
        email: accounts.snapshot().email,
        vendor: VendorReply {
            http_status: reply.http_status,
            code: reply.vendor_code,
            revoked: reply.is_ok(),
            message: reply.vendor_message,
        },
    })
}

pub fn api() -> Vec<rocket::route::Route> {
    routes![
        bind_by_serial,
        bind_by_qr,
        unbind_by_serial,
        remembered,
        remember,
        sign_out
    ]
}
