use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use proview_core::cloud::bind::BindStatus;
use proview_core::cloud::bind::{parse_qr, serial_ok, AddDevice, BindClient, BindFailure};
use proview_core::cloud::claims;
use proview_core::cloud::classify;
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
const REMEMBER: &str = "pvw_remember";
const REMEMBER_SECONDS: i64 = 30 * 24 * 60 * 60;

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
    #[serde(default)]
    pub browser: String,
}

pub struct AccountStore {
    path: Option<PathBuf>,
    inner: RwLock<Remembered>,
}

fn same(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() || left.is_empty() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
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

    fn write(&self, remembered: &Remembered) -> std::io::Result<()> {
        *self.inner.write().expect("account store poisoned") = remembered.clone();
        let path = match &self.path {
            Some(path) => path,
            None => return Ok(()),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_vec_pretty(remembered)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(path)?;
        file.write_all(&raw)?;
        file.sync_all()
    }

    pub fn remember(
        &self,
        email: &str,
        token: &str,
        expires_at: Option<i64>,
        browser: &str,
    ) -> std::io::Result<()> {
        self.write(&Remembered {
            email: email.trim().to_string(),
            access_token: token.to_string(),
            expires_at,
            saved_at: now_seconds(),
            browser: browser.to_string(),
        })
    }

    pub fn forget_token(&self) -> std::io::Result<()> {
        let mut current = self.snapshot();
        current.access_token.clear();
        current.expires_at = None;
        current.browser.clear();
        current.saved_at = now_seconds();
        self.write(&current)
    }
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

fn live_for(remembered: &Remembered) -> Option<i64> {
    if remembered.access_token.trim().is_empty() {
        return None;
    }
    let expires = remembered.expires_at?;
    Some(expires - 60)
}

fn cloud_failure(error: &CloudError) -> Failure {
    match error {
        CloudError::Credentials => fail(Status::Unauthorized, "the cloud refused the sign-in"),
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

fn bind_failure(failure: BindFailure) -> Failure {
    match failure {
        BindFailure::Vendor { code, message } => {
            let named = format!("{message} ({code})");
            match classify(code, Some(message.as_str())) {
                CloudError::Rejected(_) => fail_code(Status::BadRequest, "bind_rejected", named),
                other => cloud_failure(&other),
            }
        }
        BindFailure::Cloud(error) => cloud_failure(&error),
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
pub struct UnbindBody {
    pub sn: String,
    #[serde(default)]
    pub confirm: String,
}

#[derive(Deserialize)]
pub struct QrBody {
    pub payload: String,
    #[serde(default)]
    pub password: Option<String>,
}

fn serial_or_fail(serial: &str) -> Result<String, Failure> {
    let trimmed = serial.trim();
    if !serial_ok(trimmed) {
        return Err(fail_code(
            Status::BadRequest,
            "bad_serial",
            "a device serial is 8 to 32 letters and digits, as it is printed on the camera",
        ));
    }
    Ok(trimmed.to_string())
}

fn request_for(body: &BindBody, session: &VendorSession) -> Result<AddDevice, Failure> {
    let serial = serial_or_fail(&body.sn)?;
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
        serial: serial.clone(),
        username: if body.username.trim().is_empty() {
            "admin".to_string()
        } else {
            body.username.trim().to_string()
        },
        password: body.password.clone(),
        account_id,
        nickname: if body.nickname.trim().is_empty() {
            serial
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
        .map_err(bind_failure)?;
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

const SHARE_KEY_WINDOW: Duration = Duration::from_secs(12);

async fn shared_payload(
    key: &str,
    session: &VendorSession,
    cfg: &State<Config>,
) -> Result<String, Failure> {
    let token = session.token.as_deref().ok_or_else(|| {
        fail(
            Status::Unauthorized,
            "a share code is read from the cloud; sign in with the cloud account",
        )
    })?;
    let client = proview_core::cloud::attributes::AttributeClient::with_base(
        Identity::resolve(),
        cfg.cloud_base.clone(),
    );
    let blob =
        match tokio::time::timeout(SHARE_KEY_WINDOW, client.share_info_of_key(token, key)).await {
            Ok(result) => result.map_err(|error| cloud_failure(&error))?,
            Err(_) => {
                return Err(fail(
                    Status::GatewayTimeout,
                    format!(
                        "the cloud did not answer the share code lookup within {} ms",
                        SHARE_KEY_WINDOW.as_millis()
                    ),
                ))
            }
        };
    Ok(format!("https://d.xmeye.net/CSee?shareInfo={blob}"))
}

#[post("/devices/bind/qr", data = "<body>")]
async fn bind_by_qr(
    body: Json<QrBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<BindReply>, Failure> {
    let (_, session) = live_session(store, cookies)?;
    let payload = match proview_core::cloud::attributes::share_key_of(&body.payload) {
        Some(key) => shared_payload(&key, &session, cfg).await?,
        None => body.payload.clone(),
    };
    let device = parse_qr(&payload)
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

fn confirms(typed: &str, serial: &str, nickname: &str) -> bool {
    let typed = typed.trim().to_lowercase();
    !typed.is_empty()
        && (typed == serial.trim().to_lowercase()
            || (!nickname.trim().is_empty() && typed == nickname.trim().to_lowercase()))
}

#[post("/devices/unbind", data = "<body>")]
async fn unbind_by_serial(
    body: Json<UnbindBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<BindReply>, Failure> {
    let (_, session) = live_session(store, cookies)?;
    let requested = serial_or_fail(&body.sn)?;
    let token = session.token.as_deref().ok_or_else(|| {
        fail(
            Status::Unauthorized,
            "session carries no cloud token; sign in again",
        )
    })?;
    let devices = super::cloud_clips::envelope_for(cfg)
        .device_list(token)
        .await
        .map_err(|error| cloud_failure(&error))?;
    let (serial, nickname) = match devices
        .iter()
        .find(|device| device.uuid.trim().eq_ignore_ascii_case(&requested))
    {
        Some(device) => (device.uuid.trim().to_string(), device.nickname.clone()),
        None => {
            return Err(fail_code(
                Status::Conflict,
                "not_on_account",
                format!("{requested} is not in this account's device list; nothing was removed"),
            ))
        }
    };
    if !confirms(&body.confirm, &serial, &nickname) {
        return Err(fail_code(
            Status::Conflict,
            "confirm_mismatch",
            "type the camera's name or serial exactly to confirm the removal; nothing was removed",
        ));
    }
    let client = BindClient::new(Identity::resolve(), cfg.cloud_base.clone());
    let code = client
        .remove_device(token, &serial)
        .await
        .map_err(bind_failure)?;
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

fn browser_matches(accounts: &State<AccountStore>, cookies: &CookieJar<'_>) -> bool {
    let stored = accounts.snapshot().browser;
    match cookies.get(REMEMBER) {
        Some(cookie) => same(cookie.value(), &stored),
        None => false,
    }
}

fn remember_cookie(secret: &str) -> Cookie<'static> {
    Cookie::build((REMEMBER, secret.to_string()))
        .http_only(true)
        .same_site(SameSite::Strict)
        .path("/")
        .max_age(rocket::time::Duration::seconds(REMEMBER_SECONDS))
        .build()
}

#[get("/account/remembered")]
fn remembered(
    accounts: &State<AccountStore>,
    store: &State<Store>,
    cfg: &State<Config>,
    cookies: &CookieJar<'_>,
) -> Json<RememberReply> {
    if !browser_matches(accounts, cookies) {
        return Json(RememberReply {
            ok: true,
            email: String::new(),
            restored: false,
            message: "no remembered account in this browser".to_string(),
        });
    }
    let saved = accounts.snapshot();
    if let Some(id) = cookies
        .get_private(COOKIE)
        .map(|cookie| cookie.value().to_string())
    {
        if store.get(&id).is_some() {
            return Json(RememberReply {
                ok: true,
                email: saved.email,
                restored: true,
                message: "already signed in".to_string(),
            });
        }
    }
    let live = match live_for(&saved) {
        Some(value) => value,
        None => {
            return Json(RememberReply {
                ok: true,
                email: saved.email,
                restored: false,
                message: "the remembered session expired; sign in again".to_string(),
            })
        }
    };
    let remaining = live - now_seconds();
    if remaining <= 0 {
        return Json(RememberReply {
            ok: true,
            email: saved.email,
            restored: false,
            message: "the remembered session expired; sign in again".to_string(),
        });
    }
    let ttl = (cfg.session_ttl_secs as i64).min(remaining);
    let id = new_id();
    store.put(
        id.clone(),
        VendorSession {
            account: saved.email.clone(),
            token: Some(saved.access_token.clone()),
        },
        Duration::from_secs(ttl.max(1) as u64),
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
    let secret = new_id() + &new_id();
    accounts
        .remember(&session.account, token, expires_at, &secret)
        .map_err(|error| {
            fail_code(
                Status::InternalServerError,
                "remember_write_failed",
                format!("the remembered account could not be written: {error}"),
            )
        })?;
    cookies.add(remember_cookie(&secret));
    Ok(Json(RememberReply {
        ok: true,
        email: session.account,
        restored: true,
        message: "this browser will be signed in next time".to_string(),
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
    modes: &State<crate::config::ModeStore>,
    cloud: &State<crate::routes::cloud::CloudState>,
    cfg: &State<Config>,
    cookies: &CookieJar<'_>,
) -> Result<Json<SignOutBody>, Failure> {
    let saved = accounts.snapshot();
    let session_token = cookies
        .get_private(COOKIE)
        .and_then(|cookie| store.get(cookie.value()))
        .and_then(|session| session.token);
    let authorised = session_token.is_some() || browser_matches(accounts, cookies);
    if !authorised {
        return Ok(Json(SignOutBody {
            ok: true,
            message: "this browser has no session to sign out".to_string(),
            email: String::new(),
            vendor: VendorReply {
                http_status: 0,
                code: None,
                message: "nothing to revoke".to_string(),
                revoked: false,
            },
        }));
    }
    let token = session_token.unwrap_or_else(|| saved.access_token.clone());
    if let Some(cookie) = cookies.get_private(COOKIE) {
        store.remove(cookie.value());
        modes.forget(cookie.value());
        cloud.forget_devices(cookie.value());
        cookies.remove_private(cookie);
    }
    if !token.trim().is_empty() {
        store.remove_where(|session| session.token.as_deref() == Some(token.as_str()));
    }
    accounts.forget_token().map_err(|error| {
        fail_code(
            Status::InternalServerError,
            "signout_write_failed",
            format!("the remembered token could not be cleared on disk: {error}"),
        )
    })?;
    cookies.remove(Cookie::from(REMEMBER));
    let reply = if token.trim().is_empty() {
        proview_core::cloud::SignOutReply {
            http_status: 0,
            vendor_code: None,
            vendor_message: "no vendor token to revoke".to_string(),
        }
    } else {
        BindClient::new(Identity::resolve(), cfg.cloud_base.clone())
            .vendor_sign_out(&token)
            .await
    };
    Ok(Json(SignOutBody {
        ok: true,
        message: "signed out here; the vendor reply is in vendor".to_string(),
        email: accounts.snapshot().email,
        vendor: VendorReply {
            http_status: reply.http_status,
            code: reply.vendor_code,
            revoked: reply.is_ok(),
            message: reply.vendor_message,
        },
    }))
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
