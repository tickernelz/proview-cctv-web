use proview_core::cloud::new_session_id;
use proview_core::cloud::{
    AlarmClient, AlarmEntry, AlarmPage, AlarmQuery, CloudDevice, CloudError,
};
use rocket::http::{ContentType, Cookie, CookieJar, SameSite, Status};
use rocket::response::content::RawJavaScript;
use rocket::serde::json::Json;
use rocket::serde::json::Json as JsonBody;
use rocket::State;
use serde::{Deserialize, Serialize};

use super::cloud::{cloud_failure, fail, session_id, CloudState, StatusBody};

pub const AUTH_KEYS: [&str; 6] = ["pms", "css", "dss", "tps", "rps", "p2P"];
pub const ALARM_COOKIE: &str = "pvw_alarm";

type Failure = (Status, Json<StatusBody>);

#[derive(Serialize)]
pub struct AlarmList {
    pub serial: String,
    pub name: String,
    pub total: u32,
    pub finished: bool,
    pub entries: Vec<AlarmEntry>,
    pub latency_ms: u64,
    pub queried_at: u64,
    pub poll_seconds: u64,
}

pub fn routes() -> Vec<rocket::route::Route> {
    routes![list, picture, connect, disconnect]
}

#[derive(Deserialize)]
pub struct ConnectBody {
    email: String,
    password: String,
}

#[post("/cloud/alarms/session", data = "<body>")]
pub async fn connect(
    body: JsonBody<ConnectBody>,
    state: &State<CloudState>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StatusBody>, Failure> {
    let email = body.email.trim();
    if email.is_empty() || body.password.is_empty() {
        return Err(fail(Status::BadRequest, "email and password are required"));
    }
    let tokens = state
        .envelope()
        .login_envelope(email, &body.password)
        .await
        .map_err(|error| cloud_failure(&error))?;
    let id = new_session_id();
    state.keep_mobile(&id, tokens.access);
    cookies.add_private(
        Cookie::build((ALARM_COOKIE, id))
            .http_only(true)
            .same_site(SameSite::Strict)
            .build(),
    );
    Ok(Json(StatusBody {
        ok: true,
        message: "the alarm centre is connected".to_string(),
    }))
}

#[post("/cloud/alarms/signout")]
pub fn disconnect(state: &State<CloudState>, cookies: &CookieJar<'_>) -> Json<StatusBody> {
    if let Some(cookie) = cookies.get_private(ALARM_COOKIE) {
        state.forget_mobile(cookie.value());
        cookies.remove_private(cookie);
    }
    Json(StatusBody {
        ok: true,
        message: "the alarm centre is disconnected".to_string(),
    })
}

pub fn files() -> Vec<rocket::route::Route> {
    routes![alarms_js]
}

#[get("/alarms.js")]
fn alarms_js() -> RawJavaScript<&'static str> {
    RawJavaScript(include_str!("../../../../web/alarms.js"))
}

#[get("/cloud/alarms?<sn>&<start>&<end>&<hours>&<limit>")]
pub async fn list(
    sn: String,
    start: Option<String>,
    end: Option<String>,
    hours: Option<i64>,
    limit: Option<u32>,
    state: &State<CloudState>,
    cookies: &CookieJar<'_>,
) -> Result<Json<AlarmList>, Failure> {
    let token = alarm_token(state, cookies)?;
    let device = lookup(state, &token, sn.trim()).await?;
    let page = query(&device, window(start, end, hours), limit).await?;
    Ok(Json(AlarmList {
        serial: device.uuid.clone(),
        name: device.display_name().to_string(),
        total: page.total,
        finished: page.finished,
        entries: page.entries,
        latency_ms: page.latency_ms,
        queried_at: page.queried_at,
        poll_seconds: poll_seconds(),
    }))
}

#[get("/cloud/alarms/picture?<sn>&<object>")]
pub async fn picture(
    sn: String,
    object: String,
    state: &State<CloudState>,
    cookies: &CookieJar<'_>,
) -> Result<(ContentType, Vec<u8>), Failure> {
    let token = alarm_token(state, cookies)?;
    let device = lookup(state, &token, sn.trim()).await?;
    let picture = AlarmClient::from_env()
        .picture(&device.uuid, &auth_code(&device), object.trim())
        .await
        .map_err(|error| cloud_failure(&error))?;
    Ok((ContentType::JPEG, picture))
}

fn alarm_token(state: &CloudState, cookies: &CookieJar<'_>) -> Result<String, Failure> {
    if let Ok(id) = session_id(cookies) {
        if let Some(token) = state.mobile_token(&id) {
            return Ok(token);
        }
    }
    if let Some(cookie) = cookies.get_private(ALARM_COOKIE) {
        if let Some(token) = state.mobile_token(cookie.value()) {
            return Ok(token);
        }
    }
    Err(fail(
        Status::Unauthorized,
        "the alarm centre needs the phone account once: connect it from the alarm panel",
    ))
}

async fn lookup(state: &CloudState, token: &str, serial: &str) -> Result<CloudDevice, Failure> {
    if serial.is_empty() {
        return Err(fail(Status::BadRequest, "a device serial is required"));
    }
    let devices = state
        .envelope()
        .device_list(token)
        .await
        .map_err(|error| cloud_failure(&error))?;
    devices
        .into_iter()
        .find(|device| device.uuid.eq_ignore_ascii_case(serial))
        .ok_or_else(|| {
            fail(
                Status::NotFound,
                format!("{serial} is not a device on this account"),
            )
        })
}

fn auth_codes(device: &CloudDevice) -> Vec<String> {
    let mut codes: Vec<String> = Vec::new();
    for key in AUTH_KEYS {
        let Some(value) = device.extra.get(key).and_then(|value| value.as_str()) else {
            continue;
        };
        let code = value.trim().to_string();
        if !code.is_empty() && !codes.contains(&code) {
            codes.push(code);
        }
    }
    codes
}

fn auth_code(device: &CloudDevice) -> String {
    auth_codes(device).into_iter().next().unwrap_or_default()
}

async fn query(
    device: &CloudDevice,
    window: (String, String),
    limit: Option<u32>,
) -> Result<AlarmPage, Failure> {
    let codes = auth_codes(device);
    if codes.is_empty() {
        return Err(fail(
            Status::ServiceUnavailable,
            "this device has no cloud auth code, so the alarm centre cannot answer for it",
        ));
    }
    let (start, end) = window;
    let client = AlarmClient::from_env();
    let mut last = CloudError::Rejected("the alarm centre refused every auth code".to_string());
    for code in codes {
        let request = AlarmQuery {
            serial: device.uuid.clone(),
            auth_code: code,
            channel: 0,
            start: start.clone(),
            end: end.clone(),
            offset: 0,
            number: limit.unwrap_or(proview_core::cloud::alarm::PAGE_LIMIT),
        };
        match client.list(&request).await {
            Ok(page) => return Ok(page),
            Err(error) => last = error,
        }
    }
    Err(cloud_failure(&last))
}

fn window(start: Option<String>, end: Option<String>, hours: Option<i64>) -> (String, String) {
    let stamp_of = |value: Option<String>| {
        value
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    };
    if let (Some(start), Some(end)) = (stamp_of(start), stamp_of(end)) {
        return (start, end);
    }
    let hours = hours.filter(|value| *value > 0).unwrap_or(24).min(24 * 90);
    (stamp(hours * 3600), stamp(0))
}

fn stamp(seconds_back: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let stamp = now - seconds_back;
    let days = stamp.div_euclid(86_400);
    let rest = stamp.rem_euclid(86_400);
    let (year, month, day) = civil(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn poll_seconds() -> u64 {
    std::env::var("PROVIEW_ALARM_POLL_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .filter(|value: &u64| *value >= 5)
        .unwrap_or(20)
}
