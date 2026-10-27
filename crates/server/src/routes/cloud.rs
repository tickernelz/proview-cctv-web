use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use proview_core::cloud::{
    new_session_id, AlarmClient, AppCaps, CapsClient, Captcha, CloudClient, CloudDevice,
    CloudError, CloudProfile, CloudSessions, EnvelopeClient, Identity, REQUESTED_CAPS,
};
use proview_core::provider::Device;
use rocket::http::{Cookie, CookieJar, SameSite, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};

const COOKIE: &str = "pvw_cloud";

pub struct CloudState {
    client: CloudClient,
    caps: CapsClient,
    sessions: CloudSessions,
    envelope: EnvelopeClient,
    alarms: AlarmClient,
    devices: Mutex<HashMap<String, (Instant, Vec<CloudDevice>)>>,
}

pub(crate) const DEVICE_CACHE_TTL: Duration = Duration::from_secs(60);

impl CloudState {
    pub fn new() -> Self {
        let ttl = std::env::var("PROVIEW_CLOUD_SESSION_TTL")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(3600);
        Self {
            client: CloudClient::from_env(),
            caps: CapsClient::from_env(),
            sessions: CloudSessions::new(Duration::from_secs(ttl)),
            envelope: EnvelopeClient::new(Identity::resolve()),
            alarms: AlarmClient::from_env(),
            devices: Mutex::new(HashMap::new()),
        }
    }

    pub fn portal(&self) -> &str {
        self.client.base()
    }

    pub(crate) fn envelope(&self) -> &EnvelopeClient {
        &self.envelope
    }

    pub(crate) fn alarms(&self) -> &AlarmClient {
        &self.alarms
    }

    pub(crate) fn cached_devices(&self, session: &str) -> Option<Vec<CloudDevice>> {
        let mut guard = self.devices.lock().ok()?;
        guard.retain(|_, (seen, _)| seen.elapsed() < DEVICE_CACHE_TTL);
        guard.get(session).map(|(_, devices)| devices.clone())
    }

    pub(crate) fn keep_devices(&self, session: &str, devices: &[CloudDevice]) {
        if let Ok(mut guard) = self.devices.lock() {
            guard.retain(|_, (seen, _)| seen.elapsed() < DEVICE_CACHE_TTL);
            guard.insert(session.to_string(), (Instant::now(), devices.to_vec()));
        }
    }

    pub(crate) fn forget_devices(&self, session: &str) {
        if let Ok(mut guard) = self.devices.lock() {
            guard.remove(session);
        }
    }
}

impl Default for CloudState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize)]
pub struct StatusBody {
    pub ok: bool,
    pub message: String,
}

type Failure = (Status, Json<StatusBody>);

pub(crate) fn fail(status: Status, message: impl Into<String>) -> Failure {
    (
        status,
        Json(StatusBody {
            ok: false,
            message: message.into(),
        }),
    )
}

pub(crate) fn cloud_failure(error: &CloudError) -> Failure {
    tracing::warn!(%error, "cloud call failed");
    fail(status_for(error), error.to_string())
}

pub(crate) fn status_for(error: &CloudError) -> Status {
    match error {
        CloudError::Rejected(_) => Status::BadRequest,
        CloudError::Credentials => Status::Unauthorized,
        CloudError::Expired => Status::Unauthorized,
        CloudError::Unreachable(_) => Status::ServiceUnavailable,
        CloudError::Malformed(_) => Status::BadGateway,
    }
}

pub(crate) fn session_id(cookies: &CookieJar<'_>) -> Result<String, Failure> {
    cookies
        .get_private(COOKIE)
        .map(|cookie| cookie.value().to_string())
        .ok_or_else(|| fail(Status::Unauthorized, "not signed in to the cloud"))
}

pub(crate) fn restore(
    state: &CloudState,
    cookies: &CookieJar<'_>,
) -> Result<proview_core::cloud::CloudSession, Failure> {
    let id = session_id(cookies)?;
    state
        .sessions
        .get(&id)
        .ok_or_else(|| fail(Status::Unauthorized, "cloud session expired; sign in again"))
}

#[get("/cloud/portal")]
fn portal(state: &State<CloudState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "base": state.portal() }))
}

#[get("/cloud/captcha")]
async fn captcha(state: &State<CloudState>) -> Result<Json<Captcha>, Failure> {
    state
        .client
        .captcha()
        .await
        .map(Json)
        .map_err(|error| cloud_failure(&error))
}

#[derive(Deserialize)]
pub struct LoginBody {
    email: String,
    password: String,
    code: String,
    #[serde(alias = "codeToken")]
    code_token: String,
}

#[post("/cloud/session", data = "<body>")]
async fn login(
    body: Json<LoginBody>,
    state: &State<CloudState>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StatusBody>, Failure> {
    let email = body.email.trim();
    if email.is_empty() || body.password.is_empty() {
        return Err(fail(Status::BadRequest, "email and password are required"));
    }
    if body.code.trim().is_empty() || body.code_token.trim().is_empty() {
        return Err(fail(
            Status::BadRequest,
            "a verification code and its token are required",
        ));
    }
    let session = state
        .client
        .login(
            email,
            &body.password,
            body.code.trim(),
            body.code_token.trim(),
        )
        .await
        .map_err(|error| cloud_failure(&error))?;
    let id = new_session_id();
    state.sessions.put(id.clone(), session);
    cookies.add_private(
        Cookie::build((COOKIE, id))
            .http_only(true)
            .same_site(SameSite::Strict)
            .build(),
    );
    Ok(Json(StatusBody {
        ok: true,
        message: "signed in to the cloud".to_string(),
    }))
}

#[post("/cloud/logout")]
fn logout(state: &State<CloudState>, cookies: &CookieJar<'_>) -> Json<StatusBody> {
    if let Some(cookie) = cookies.get_private(COOKIE) {
        state.sessions.remove(cookie.value());
        state.forget_devices(cookie.value());
        cookies.remove_private(cookie);
    }
    Json(StatusBody {
        ok: true,
        message: "signed out".to_string(),
    })
}

#[get("/cloud/profile")]
async fn profile(
    state: &State<CloudState>,
    cookies: &CookieJar<'_>,
) -> Result<Json<CloudProfile>, Failure> {
    let session = restore(state, cookies)?;
    state
        .client
        .profile(&session)
        .await
        .map(Json)
        .map_err(|error| cloud_failure(&error))
}

#[get("/cloud/devices")]
async fn devices(
    state: &State<CloudState>,
    cookies: &CookieJar<'_>,
) -> Result<Json<Vec<Device>>, Failure> {
    let session = restore(state, cookies)?;
    state
        .client
        .devices_with_status(&session)
        .await
        .map(Json)
        .map_err(|error| cloud_failure(&error))
}

#[get("/cloud/device-caps?<sn>")]
async fn device_caps(state: &State<CloudState>, sn: String) -> Result<Json<AppCaps>, Failure> {
    let caps = state
        .caps
        .req_caps(&sn, &REQUESTED_CAPS)
        .await
        .map_err(|error| cloud_failure(&error))?;
    if !caps.is_known() {
        return Err(fail(
            Status::NotFound,
            format!("{} is not a device the cloud knows", sn.trim()),
        ));
    }
    Ok(Json(caps))
}

pub fn api() -> Vec<rocket::route::Route> {
    routes![
        portal,
        captcha,
        login,
        logout,
        profile,
        devices,
        device_caps
    ]
}
