use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use proview_core::cloud::account::{name_available, AccountClient, AccountProfile, CodePurpose};
use proview_core::cloud::{CloudError, Identity};
use proview_core::config::Config;
use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};

use super::cloud::{status_for, CloudState};
use crate::routes::{fail, restore, session_id, Failure};

const DEFAULT_WINDOW: Duration = Duration::from_secs(15);
const PENDING_TTL: Duration = Duration::from_secs(15 * 60);

pub fn routes() -> Vec<rocket::route::Route> {
    routes![
        change_password,
        forgot_code,
        forgot_check,
        forgot_reset,
        register_name,
        register_code,
        register,
        delete_code,
        delete_account
    ]
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Done {
    pub ok: bool,
    pub message: String,
    pub latency_ms: u64,
}

impl Done {
    fn after(started: Instant, message: impl Into<String>) -> Json<Self> {
        Json(Self {
            ok: true,
            message: message.into(),
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NameCheck {
    pub name: String,
    pub available: bool,
    pub message: String,
}

pub(crate) fn window() -> Duration {
    std::env::var("PROVIEW_ACCOUNT_WINDOW_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_WINDOW)
}

pub(crate) fn client(cfg: &Config) -> Arc<AccountClient> {
    static POOL: OnceLock<Mutex<HashMap<String, Arc<AccountClient>>>> = OnceLock::new();
    let pool = POOL.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = pool.lock().unwrap_or_else(|error| error.into_inner());
    guard
        .entry(cfg.cloud_base.clone())
        .or_insert_with(|| {
            Arc::new(AccountClient::with_base(
                Identity::resolve(),
                cfg.cloud_base.clone(),
            ))
        })
        .clone()
}

fn vendor_failure(stage: &str, error: &CloudError) -> Failure {
    tracing::warn!(%error, stage, "account call failed");
    fail(status_for(error), format!("{stage}: {error}"))
}

pub(crate) async fn within<T>(
    stage: &str,
    call: impl Future<Output = Result<T, CloudError>>,
) -> Result<T, (Status, String)> {
    let limit = window();
    match tokio::time::timeout(limit, call).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => {
            tracing::warn!(%error, stage, "account call failed");
            Err((status_for(&error), format!("{stage}: {error}")))
        }
        Err(_) => Err((
            Status::GatewayTimeout,
            format!(
                "the vendor cloud did not answer the {stage} step within {:.1} s",
                limit.as_secs_f64()
            ),
        )),
    }
}

async fn step<T>(
    stage: &str,
    call: impl Future<Output = Result<T, CloudError>>,
) -> Result<T, Failure> {
    within(stage, call)
        .await
        .map_err(|(status, message)| fail(status, message))
}

struct CloudSignIn {
    id: String,
    account: String,
    token: String,
}

fn cloud_sign_in(store: &State<Store>, cookies: &CookieJar<'_>) -> Result<CloudSignIn, Failure> {
    let id = session_id(cookies)?;
    let session = restore(store, &id)?;
    let token = session
        .token
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            fail(
                Status::Conflict,
                "this session is on a camera on this network; account settings need the cloud account sign-in",
            )
        })?;
    Ok(CloudSignIn {
        id,
        account: session.account,
        token,
    })
}

pub(crate) async fn envelope_profile(
    cfg: &Config,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Option<Result<AccountProfile, (Status, String)>> {
    let id = cookies.get_private("pvw_session")?.value().to_string();
    let token = store
        .get(&id)?
        .token
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())?;
    let accounts = client(cfg);
    Some(within("profile (userinfo2)", accounts.profile(&token)).await)
}

pub(crate) fn local_session(store: &State<Store>, cookies: &CookieJar<'_>) -> bool {
    cookies
        .get_private("pvw_session")
        .is_some_and(|cookie| store.get(cookie.value()).is_some())
}

#[derive(Deserialize)]
pub struct PasswordChange {
    #[serde(default)]
    current: String,
    #[serde(default)]
    next: String,
}

#[post("/account/password", data = "<body>")]
async fn change_password(
    body: Json<PasswordChange>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<Done>, Failure> {
    let signed = cloud_sign_in(store, cookies)?;
    if body.current.is_empty() || body.next.is_empty() {
        return Err(fail(
            Status::BadRequest,
            "both the current and the new password are required",
        ));
    }
    if body.current == body.next {
        return Err(fail(
            Status::BadRequest,
            "the new password is the same as the current one",
        ));
    }
    let started = Instant::now();
    let accounts = client(cfg);
    let fresh = match within(
        "password check (loginExist)",
        accounts.verify_password(&signed.account, &body.current),
    )
    .await
    {
        Ok(tokens) => tokens,
        Err((Status::Unauthorized, _)) => {
            return Err(fail(Status::Forbidden, "the current password is wrong"))
        }
        Err((status, message)) => return Err(fail(status, message)),
    };
    step(
        "password change (usercp)",
        accounts.change_password(&fresh.access, &body.next),
    )
    .await?;
    let message = match within(
        "sign-in with the new password (loginExist)",
        accounts.verify_password(&signed.account, &body.next),
    )
    .await
    {
        Ok(tokens) => {
            let mut session = restore(store, &signed.id)?;
            session.token = Some(tokens.access);
            store.put(
                signed.id.clone(),
                session,
                Duration::from_secs(cfg.session_ttl_secs),
            );
            "password changed; this session now uses the new password".to_string()
        }
        Err((_, message)) => {
            format!("password changed; signing in again with it failed ({message}), so sign out and back in")
        }
    };
    Ok(Done::after(started, message))
}

#[derive(Clone)]
struct Pending {
    user_id: Option<String>,
    checked: Option<String>,
    at: Instant,
}

fn pending() -> std::sync::MutexGuard<'static, HashMap<String, Pending>> {
    static PENDING: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    let mut guard = PENDING
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    guard.retain(|_, entry| entry.at.elapsed() < PENDING_TTL);
    guard
}

fn pending_key(cfg: &Config, mail: &str) -> String {
    format!("{}|{}", cfg.cloud_base, mail.trim().to_ascii_lowercase())
}

fn mail_of(value: &str) -> Result<String, Failure> {
    let mail = value.trim();
    let valid = mail
        .split_once('@')
        .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'));
    if !valid {
        return Err(fail(Status::BadRequest, "enter the account email address"));
    }
    Ok(mail.to_string())
}

#[derive(Deserialize)]
pub struct CodeRequest {
    #[serde(default)]
    email: String,
}

#[derive(Deserialize)]
pub struct CodeCheck {
    #[serde(default)]
    email: String,
    #[serde(default)]
    code: String,
}

#[derive(Deserialize)]
pub struct Reset {
    #[serde(default)]
    email: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    password: String,
}

#[post("/account/forgot/code", data = "<body>")]
async fn forgot_code(body: Json<CodeRequest>, cfg: &State<Config>) -> Result<Json<Done>, Failure> {
    let mail = mail_of(&body.email)?;
    let started = Instant::now();
    let user_id = step(
        "send code (scmail)",
        client(cfg).send_code(&mail, CodePurpose::Forgot),
    )
    .await?;
    tracing::info!(
        user_id_known = user_id.is_some(),
        "forgot-password code requested"
    );
    pending().insert(
        pending_key(cfg, &mail),
        Pending {
            user_id,
            checked: None,
            at: Instant::now(),
        },
    );
    Ok(Done::after(
        started,
        "a verification code was sent to that email",
    ))
}

#[post("/account/forgot/check", data = "<body>")]
async fn forgot_check(body: Json<CodeCheck>, cfg: &State<Config>) -> Result<Json<Done>, Failure> {
    let mail = mail_of(&body.email)?;
    let code = body.code.trim().to_string();
    if code.is_empty() {
        return Err(fail(Status::BadRequest, "enter the verification code"));
    }
    let key = pending_key(cfg, &mail);
    if !pending().contains_key(&key) {
        return Err(fail(
            Status::Conflict,
            "request a verification code for this email first",
        ));
    }
    let started = Instant::now();
    step("check code (usercpc)", client(cfg).check_code(&mail, &code)).await?;
    if let Some(entry) = pending().get_mut(&key) {
        entry.checked = Some(code);
    }
    Ok(Done::after(
        started,
        "the code is correct; choose a new password",
    ))
}

#[post("/account/forgot/reset", data = "<body>")]
async fn forgot_reset(body: Json<Reset>, cfg: &State<Config>) -> Result<Json<Done>, Failure> {
    let mail = mail_of(&body.email)?;
    if body.password.is_empty() {
        return Err(fail(Status::BadRequest, "enter a new password"));
    }
    let key = pending_key(cfg, &mail);
    let entry = pending().get(&key).cloned().ok_or_else(|| {
        fail(
            Status::Conflict,
            "request a verification code for this email first",
        )
    })?;
    let code = body.code.trim();
    if entry.checked.as_deref() != Some(code) {
        return Err(fail(
            Status::Conflict,
            "check the verification code before setting a new password",
        ));
    }
    let user_id = entry.user_id.ok_or_else(|| {
        fail(
            Status::BadGateway,
            "the cloud did not return an account id with the code; request a new code",
        )
    })?;
    let started = Instant::now();
    step(
        "reset password (fpmail)",
        client(cfg).reset_password(&user_id, &mail, code, &body.password),
    )
    .await?;
    pending().remove(&key);
    Ok(Done::after(
        started,
        "the password was reset; sign in with the new password",
    ))
}

#[get("/account/register/name?<name>")]
async fn register_name(
    name: Option<String>,
    cfg: &State<Config>,
) -> Result<Json<NameCheck>, Failure> {
    let name = name.unwrap_or_default().trim().to_string();
    if name.is_empty() {
        return Err(fail(Status::BadRequest, "enter a username"));
    }
    let reply = step("name check (usercheck)", client(cfg).name_check(&name)).await?;
    match name_available(&reply) {
        Some(available) => Ok(Json(NameCheck {
            message: if available {
                "this username is free".to_string()
            } else {
                "this username is already taken".to_string()
            },
            name,
            available,
        })),
        None => Err(vendor_failure(
            "name check (usercheck)",
            &CloudError::Rejected(reply.message()),
        )),
    }
}

#[post("/account/register/code", data = "<body>")]
async fn register_code(
    body: Json<CodeRequest>,
    cfg: &State<Config>,
) -> Result<Json<Done>, Failure> {
    let mail = mail_of(&body.email)?;
    let started = Instant::now();
    step(
        "send code (scmail)",
        client(cfg).send_code(&mail, CodePurpose::Register),
    )
    .await?;
    Ok(Done::after(
        started,
        "a verification code was sent to that email",
    ))
}

#[derive(Deserialize)]
pub struct Registration {
    #[serde(default)]
    username: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    code: String,
}

#[post("/account/register", data = "<body>")]
async fn register(body: Json<Registration>, cfg: &State<Config>) -> Result<Json<Done>, Failure> {
    let mail = mail_of(&body.email)?;
    if body.username.trim().is_empty() || body.password.is_empty() || body.code.trim().is_empty() {
        return Err(fail(
            Status::BadRequest,
            "username, password and verification code are required",
        ));
    }
    let started = Instant::now();
    step(
        "register (userregMail)",
        client(cfg).register(&body.username, &body.password, &mail, &body.code),
    )
    .await?;
    Ok(Done::after(
        started,
        "the account was created; sign in with it",
    ))
}

#[post("/account/delete/code")]
async fn delete_code(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<Done>, Failure> {
    let signed = cloud_sign_in(store, cookies)?;
    let started = Instant::now();
    step(
        "request deletion code (userecancel)",
        client(cfg).cancel(&signed.token, ""),
    )
    .await?;
    Ok(Done::after(
        started,
        "a verification code was sent to the account email",
    ))
}

#[derive(Deserialize)]
pub struct Deletion {
    #[serde(default)]
    account: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    confirm: bool,
}

#[post("/account/delete", data = "<body>")]
async fn delete_account(
    body: Json<Deletion>,
    cfg: &State<Config>,
    store: &State<Store>,
    cloud: &State<CloudState>,
    cookies: &CookieJar<'_>,
) -> Result<Json<Done>, Failure> {
    let signed = cloud_sign_in(store, cookies)?;
    if !body.confirm {
        return Err(fail(
            Status::BadRequest,
            "confirm that the account and everything on it will be deleted",
        ));
    }
    if !body
        .account
        .trim()
        .eq_ignore_ascii_case(signed.account.trim())
    {
        return Err(fail(
            Status::BadRequest,
            "the typed account does not match the signed-in account",
        ));
    }
    let code = body.code.trim();
    if code.is_empty() {
        return Err(fail(Status::BadRequest, "enter the verification code"));
    }
    let started = Instant::now();
    step(
        "delete account (userecancel)",
        client(cfg).cancel(&signed.token, code),
    )
    .await?;
    store.remove(&signed.id);
    cloud.forget_devices(&signed.id);
    if let Some(cookie) = cookies.get_private("pvw_session") {
        cookies.remove_private(cookie);
    }
    Ok(Done::after(started, "the cloud account was deleted"))
}
