use std::io::Cursor;

use proview_core::cloud::alarm::{AlarmDay, AlarmTarget, DeleteScope, EVENT_FILTERS};
use proview_core::cloud::{
    claims, AlarmClient, AlarmEntry, AlarmPage, AlarmQuery, CloudDevice, CloudError, SignRequest,
};
use proview_core::session::Store;
use rocket::http::{ContentType, CookieJar, Status};
use rocket::request::Request;
use rocket::response::{Responder, Response};
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};

use super::cloud::{status_for, CloudState};
use crate::routes::{fail, restore, session_id, Failure};

pub const AUTH_KEYS: [&str; 6] = ["pms", "css", "dss", "tps", "rps", "p2P"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlarmList {
    pub serial: String,
    pub name: String,
    pub total: u32,
    pub finished: bool,
    pub entries: Vec<AlarmEntry>,
    pub latency_ms: u64,
    pub queried_at: u64,
    pub poll_seconds: u64,
    pub can_delete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_reason: Option<String>,
}

pub fn routes() -> Vec<rocket::route::Route> {
    routes![list, picture, filters, delete, calendar, subscription]
}

const DEFAULT_WINDOW_MS: u64 = 8_000;

fn alarm_window() -> std::time::Duration {
    let millis = std::env::var("PROVIEW_ALARM_WINDOW_MS")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .filter(|value: &u64| *value > 0)
        .unwrap_or(DEFAULT_WINDOW_MS);
    std::time::Duration::from_millis(millis)
}

async fn within<T>(
    stage: &str,
    work: impl std::future::Future<Output = Result<T, Failure>>,
) -> Result<T, Failure> {
    within_window(stage, alarm_window(), work).await
}

async fn within_window<T>(
    stage: &str,
    window: std::time::Duration,
    work: impl std::future::Future<Output = Result<T, Failure>>,
) -> Result<T, Failure> {
    match tokio::time::timeout(window, work).await {
        Ok(result) => result,
        Err(_) => Err(fail(
            Status::GatewayTimeout,
            format!(
                "the alarm centre did not answer the {stage} request within {} ms",
                window.as_millis()
            ),
        )),
    }
}

fn event_filter(event: Option<String>) -> Result<String, Failure> {
    let wanted = event.unwrap_or_default().trim().to_string();
    if wanted.is_empty() || EVENT_FILTERS.iter().any(|(id, _)| *id == wanted) {
        return Ok(wanted);
    }
    Err(fail(
        Status::BadRequest,
        format!("{wanted:?} is not an alarm type this camera reports"),
    ))
}

#[derive(Serialize)]
pub struct FilterOption {
    pub id: &'static str,
    pub label: &'static str,
}

#[get("/cloud/alarms/filters")]
pub fn filters() -> Json<Vec<FilterOption>> {
    Json(
        EVENT_FILTERS
            .iter()
            .map(|(id, label)| FilterOption { id, label })
            .collect(),
    )
}

#[derive(Deserialize)]
pub struct DeleteBody {
    pub sn: String,
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub all: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteReply {
    pub serial: String,
    pub all: bool,
    pub deleted: usize,
    pub latency_ms: u64,
}

pub const DELETE_PERMISSION: &str = "DP_DeleteAlarmInfo";

fn field_text(device: &CloudDevice, key: &str) -> String {
    match device.extra.get(key) {
        Some(serde_json::Value::String(text)) => text.trim().to_string(),
        Some(serde_json::Value::Number(number)) => number.to_string(),
        _ => String::new(),
    }
}

pub fn received_share(device: &CloudDevice) -> bool {
    ["shareId", "deviceOwnerId", "shareAuthToken"]
        .iter()
        .any(|key| !field_text(device, key).is_empty())
        || device.extra.contains_key("devPermissions")
        || device.extra.contains_key("permissions")
}

fn granted(device: &CloudDevice, permission: &str) -> bool {
    let listed = field_text(device, "devPermissions");
    if listed.split(',').any(|item| item.trim() == permission) {
        return true;
    }
    device
        .extra
        .get("permissions")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items.iter().any(|item| {
                item.get("label").and_then(serde_json::Value::as_str) == Some(permission)
                    && item
                        .get("enabled")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

pub fn delete_refusal(device: &CloudDevice) -> Option<String> {
    if !received_share(device) || granted(device, DELETE_PERMISSION) {
        return None;
    }
    Some(format!(
        "{} was shared with this account without the right to delete alarm messages; only its owner can delete them",
        device.display_name()
    ))
}

pub fn delete_scope(body: &DeleteBody) -> Result<DeleteScope, Failure> {
    let ids: Vec<String> = body
        .ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    match (body.all, ids.is_empty()) {
        (true, true) => Ok(DeleteScope::All),
        (true, false) => Err(fail(
            Status::BadRequest,
            "clear all takes no alarm ids; send either ids or all",
        )),
        (false, true) => Err(fail(
            Status::BadRequest,
            "select at least one alarm to delete, or ask to clear all",
        )),
        (false, false) => {
            if let Some(bad) = ids
                .iter()
                .find(|id| !proview_core::cloud::alarm::alarm_id_ok(id))
            {
                return Err(fail(
                    Status::BadRequest,
                    format!("{bad:?} is not an alarm id"),
                ));
            }
            Ok(DeleteScope::Ids(ids))
        }
    }
}

#[post("/cloud/alarms/delete", data = "<body>")]
pub async fn delete(
    body: Json<DeleteBody>,
    state: &State<CloudState>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<DeleteReply>, Failure> {
    let scope = delete_scope(&body)?;
    let session_id = session_id(cookies)?;
    let token = cloud_token(store, &session_id)?;
    let started = std::time::Instant::now();
    let device = within(
        "device lookup",
        lookup(state, &session_id, &token, body.sn.trim()),
    )
    .await?;
    if let Some(reason) = delete_refusal(&device) {
        return Err(fail(Status::Forbidden, reason));
    }
    let user = account_of(&token);
    within("delete", async {
        with_codes(&device, |code| {
            let target = AlarmTarget {
                serial: device.uuid.clone(),
                auth_code: code,
                user_id: user.clone(),
            };
            let scope = scope.clone();
            async move { state.alarms().delete(&target, &scope).await }
        })
        .await
    })
    .await?;
    Ok(Json(DeleteReply {
        serial: device.uuid.clone(),
        all: matches!(scope, DeleteScope::All),
        deleted: match &scope {
            DeleteScope::Ids(ids) => ids.len(),
            DeleteScope::All => 0,
        },
        latency_ms: started.elapsed().as_millis() as u64,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarReply {
    pub serial: String,
    pub month: String,
    pub days: Vec<AlarmDay>,
    pub latency_ms: u64,
}

#[get("/cloud/alarms/calendar?<sn>&<month>&<channel>")]
pub async fn calendar(
    sn: String,
    month: String,
    channel: Option<u32>,
    state: &State<CloudState>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<CalendarReply>, Failure> {
    let month = month.trim().to_string();
    if !proview_core::cloud::alarm::month_ok(&month) {
        return Err(fail(Status::BadRequest, "month must be in YYYY-MM form"));
    }
    let session_id = session_id(cookies)?;
    let token = cloud_token(store, &session_id)?;
    let started = std::time::Instant::now();
    let device = within(
        "device lookup",
        lookup(state, &session_id, &token, sn.trim()),
    )
    .await?;
    let user = account_of(&token);
    let channel = channel.unwrap_or_default();
    let days = within("month calendar", async {
        with_codes(&device, |code| {
            let target = AlarmTarget {
                serial: device.uuid.clone(),
                auth_code: code,
                user_id: user.clone(),
            };
            let month = month.clone();
            async move { state.alarms().calendar(&target, &month, channel).await }
        })
        .await
    })
    .await?;
    Ok(Json(CalendarReply {
        serial: device.uuid.clone(),
        month,
        days,
        latency_ms: started.elapsed().as_millis() as u64,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionReply {
    pub serial: String,
    pub subscribed: bool,
    pub count: usize,
    pub asked: usize,
    pub note: String,
    pub latency_ms: u64,
}

#[get("/cloud/alarms/subscription?<sn>&<tokens>")]
pub async fn subscription(
    sn: String,
    tokens: Option<String>,
    state: &State<CloudState>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<SubscriptionReply>, Failure> {
    let asked: Vec<String> = tokens
        .unwrap_or_default()
        .split("&&")
        .flat_map(|part| part.split(','))
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
        .collect();
    let session_id = session_id(cookies)?;
    let token = cloud_token(store, &session_id)?;
    let started = std::time::Instant::now();
    let device = within(
        "device lookup",
        lookup(state, &session_id, &token, sn.trim()),
    )
    .await?;
    let user = account_of(&token);
    let found = within("push subscription", async {
        with_codes(&device, |code| {
            let target = AlarmTarget {
                serial: device.uuid.clone(),
                auth_code: code,
                user_id: user.clone(),
            };
            let asked = asked.clone();
            async move { state.alarms().subscription(&target, &asked).await }
        })
        .await
    })
    .await?;
    let subscribed = !asked.is_empty() && asked.iter().all(|wanted| found.contains(wanted));
    let note = if asked.is_empty() {
        "no push token was given, so this only counts the phones the alarm centre pushes this camera to; the desktop app reads alarms by polling and registers no push token".to_string()
    } else if subscribed {
        "every given push token is subscribed to this camera".to_string()
    } else {
        "the alarm centre does not push this camera to every given token".to_string()
    };
    Ok(Json(SubscriptionReply {
        serial: device.uuid.clone(),
        subscribed,
        count: found.len(),
        asked: asked.len(),
        note,
        latency_ms: started.elapsed().as_millis() as u64,
    }))
}

async fn with_codes<T, F, Fut>(device: &CloudDevice, mut work: F) -> Result<T, Failure>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<T, CloudError>>,
{
    let codes = auth_codes(device);
    if codes.is_empty() {
        return Err(fail(
            Status::ServiceUnavailable,
            "this device has no cloud auth code, so the alarm centre cannot answer for it",
        ));
    }
    let mut last = CloudError::Rejected("the alarm centre refused every auth code".to_string());
    for code in codes {
        match work(code).await {
            Ok(value) => return Ok(value),
            Err(CloudError::Rejected(reason)) if is_authcode_refusal(&reason) => {
                last = CloudError::Rejected(reason);
            }
            Err(error) => return Err(vendor_failure(&error)),
        }
    }
    Err(vendor_failure(&last))
}

#[get("/cloud/alarms?<sn>&<start>&<end>&<limit>&<event>")]
#[allow(clippy::too_many_arguments)]
pub async fn list(
    sn: String,
    start: Option<String>,
    end: Option<String>,
    limit: Option<u32>,
    event: Option<String>,
    state: &State<CloudState>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<AlarmList>, Failure> {
    let event = event_filter(event)?;
    let session_id = session_id(cookies)?;
    let token = cloud_token(store, &session_id)?;
    let device = lookup(state, &session_id, &token, sn.trim()).await?;
    let user = if event.is_empty() {
        String::new()
    } else {
        account_of(&token)
    };
    let page = query(state, &device, window(start, end), limit, &event, &user).await?;
    Ok(Json(AlarmList {
        serial: device.uuid.clone(),
        name: device.display_name().to_string(),
        total: page.total,
        finished: page.finished,
        entries: page.entries,
        latency_ms: page.latency_ms,
        queried_at: page.queried_at,
        poll_seconds: poll_seconds(),
        can_delete: delete_refusal(&device).is_none(),
        delete_reason: delete_refusal(&device),
    }))
}

const PICTURE_CACHE: &str = "private, max-age=86400, immutable";

pub struct Picture(Vec<u8>);

impl<'r> Responder<'r, 'r> for Picture {
    fn respond_to(self, _: &'r Request<'_>) -> rocket::response::Result<'r> {
        Response::build()
            .header(ContentType::JPEG)
            .raw_header("Cache-Control", PICTURE_CACHE)
            .sized_body(self.0.len(), Cursor::new(self.0))
            .ok()
    }
}

#[get("/cloud/alarms/picture?<sn>&<object>&<bucket>")]
pub async fn picture(
    sn: String,
    object: String,
    bucket: Option<String>,
    state: &State<CloudState>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Picture, Failure> {
    let session_id = session_id(cookies)?;
    let token = cloud_token(store, &session_id)?;
    let device = lookup(state, &session_id, &token, sn.trim()).await?;
    let request = SignRequest {
        serial: device.uuid.clone(),
        auth_code: auth_code(&device),
        user_id: account_of(&token),
        object: owned_object(&object, &device.uuid)?,
        bucket: bucket_name(bucket.as_deref())?,
    };
    let picture = state
        .css()
        .picture(&request)
        .await
        .map_err(|error| vendor_failure(&error))?;
    Ok(Picture(picture))
}

fn owned_object(object: &str, serial: &str) -> Result<String, Failure> {
    let name = object_name(object)?;
    let stamp = format!("_{}_", serial.trim().to_ascii_lowercase());
    if !name.to_ascii_lowercase().contains(&stamp) {
        return Err(fail(
            Status::BadRequest,
            "that picture does not belong to this device",
        ));
    }
    Ok(name)
}

fn account_of(token: &str) -> String {
    claims(token)
        .and_then(|claims| claims.account)
        .unwrap_or_default()
}

fn bucket_name(bucket: Option<&str>) -> Result<String, Failure> {
    let name = bucket.unwrap_or_default().trim();
    let shaped = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || matches!(letter, '_' | '-'));
    if !shaped {
        return Err(fail(
            Status::BadRequest,
            "the alarm entry carries no storage bucket",
        ));
    }
    Ok(name.to_string())
}

fn vendor_failure(error: &CloudError) -> Failure {
    tracing::warn!(%error, "alarm centre call failed");
    fail(status_for(error), error.to_string())
}

fn cloud_token(store: &State<Store>, id: &str) -> Result<String, Failure> {
    let session = restore(store, id)?;
    session
        .token
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            fail(
                Status::Unauthorized,
                "alarms read the vendor cloud; sign in with the cloud account, not the camera on this network",
            )
        })
}

async fn lookup(
    state: &CloudState,
    session: &str,
    token: &str,
    serial: &str,
) -> Result<CloudDevice, Failure> {
    let serial = serial.trim();
    if serial.is_empty() {
        return Err(fail(Status::BadRequest, "a device serial is required"));
    }
    if !plain_token(serial) {
        return Err(fail(Status::BadRequest, "that is not a device serial"));
    }
    let cached = state.cached_devices(session);
    if let Some(found) = cached
        .as_ref()
        .and_then(|devices| find_device(devices, serial))
    {
        return Ok(found);
    }
    let devices = state
        .envelope()
        .device_list(token)
        .await
        .map_err(|error| vendor_failure(&error))?;
    state.keep_devices(session, &devices);
    find_device(&devices, serial).ok_or_else(|| {
        fail(
            Status::NotFound,
            format!("{serial} is not a device on this account"),
        )
    })
}

fn find_device(devices: &[CloudDevice], serial: &str) -> Option<CloudDevice> {
    devices
        .iter()
        .find(|device| device.uuid.eq_ignore_ascii_case(serial))
        .cloned()
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
    state: &CloudState,
    device: &CloudDevice,
    window: (String, String),
    limit: Option<u32>,
    event: &str,
    user: &str,
) -> Result<AlarmPage, Failure> {
    let (start, end) = window;
    let number = limit
        .unwrap_or(proview_core::cloud::alarm::PAGE_LIMIT)
        .clamp(1, proview_core::cloud::alarm::PAGE_LIMIT);
    match page_filtered(state.alarms(), device, &start, &end, number, event, user).await {
        Ok(page) => Ok(page),
        Err(AlarmQueryError::NoAuthCode) => Err(fail(
            Status::ServiceUnavailable,
            "this device has no cloud auth code, so the alarm centre cannot answer for it",
        )),
        Err(AlarmQueryError::Cloud(error)) => Err(vendor_failure(&error)),
    }
}

pub(crate) enum AlarmQueryError {
    NoAuthCode,
    Cloud(CloudError),
}

pub(crate) async fn page_for(
    client: &AlarmClient,
    device: &CloudDevice,
    start: &str,
    end: &str,
    number: u32,
) -> Result<AlarmPage, AlarmQueryError> {
    page_filtered(client, device, start, end, number, "", "").await
}

async fn page_filtered(
    client: &AlarmClient,
    device: &CloudDevice,
    start: &str,
    end: &str,
    number: u32,
    event: &str,
    user: &str,
) -> Result<AlarmPage, AlarmQueryError> {
    let codes = auth_codes(device);
    if codes.is_empty() {
        return Err(AlarmQueryError::NoAuthCode);
    }
    let mut last = CloudError::Rejected("the alarm centre refused every auth code".to_string());
    for code in codes {
        let request = AlarmQuery {
            serial: device.uuid.clone(),
            auth_code: code,
            channel: 0,
            start: start.to_string(),
            end: end.to_string(),
            offset: 0,
            number,
            event: event.to_string(),
            user_id: user.to_string(),
        };
        match client.list(&request).await {
            Ok(page) => return Ok(page),
            Err(CloudError::Rejected(reason)) if is_authcode_refusal(&reason) => {
                last = CloudError::Rejected(reason);
            }
            Err(error) => return Err(AlarmQueryError::Cloud(error)),
        }
    }
    Err(AlarmQueryError::Cloud(last))
}

fn is_authcode_refusal(reason: &str) -> bool {
    reason.to_lowercase().contains("authcode")
}

fn object_name(object: &str) -> Result<String, Failure> {
    let name = object.trim();
    let shaped = !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || matches!(letter, '_' | '-' | '.'));
    if !shaped {
        return Err(fail(
            Status::BadRequest,
            "that is not an alarm picture name",
        ));
    }
    Ok(name.to_string())
}

fn plain_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || matches!(letter, '_' | '-'))
}

fn window(start: Option<String>, end: Option<String>) -> (String, String) {
    let stamp_of = |value: Option<String>| {
        value
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    };
    if let (Some(start), Some(end)) = (stamp_of(start), stamp_of(end)) {
        return (start, end);
    }
    (stamp(24 * 3600), stamp(0))
}

fn stamp(seconds_back: i64) -> String {
    stamp_at(now_seconds() - seconds_back)
}

pub(crate) fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn stamp_at(stamp: i64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_web_reads_camel_case_keys_from_the_route() {
        let body = AlarmList {
            serial: "DEV0001".into(),
            name: "Gate".into(),
            total: 2,
            finished: true,
            entries: Vec::new(),
            latency_ms: 104,
            queried_at: 7,
            poll_seconds: 20,
            can_delete: true,
            delete_reason: None,
        };
        let json = serde_json::to_value(&body).expect("alarm list json");
        for key in [
            "latencyMs",
            "pollSeconds",
            "queriedAt",
            "finished",
            "entries",
        ] {
            assert!(json.get(key).is_some(), "{key} missing from {json}");
        }
        assert!(json.get("poll_seconds").is_none());
    }

    fn body(ids: &[&str], all: bool) -> DeleteBody {
        DeleteBody {
            sn: "serial-one".into(),
            ids: ids.iter().map(|id| id.to_string()).collect(),
            all,
        }
    }

    #[test]
    fn a_delete_needs_either_ids_or_an_explicit_clear_all() {
        assert_eq!(
            delete_scope(&body(&["261028205437", " "], false)).ok(),
            Some(DeleteScope::Ids(vec!["261028205437".into()]))
        );
        assert_eq!(delete_scope(&body(&[], true)).ok(), Some(DeleteScope::All));
        assert!(delete_scope(&body(&[], false)).is_err());
        assert!(delete_scope(&body(&["  "], false)).is_err());
        assert!(delete_scope(&body(&["261028205437"], true)).is_err());
        assert!(delete_scope(&body(&["26102/8"], false)).is_err());
    }

    #[test]
    fn only_listed_alarm_types_pass_the_filter() {
        assert_eq!(event_filter(None).ok().as_deref(), Some(""));
        assert_eq!(
            event_filter(Some(" VideoMotion ".into())).ok().as_deref(),
            Some("VideoMotion")
        );
        assert!(event_filter(Some("Everything".into())).is_err());
    }

    #[test]
    fn only_an_authcode_refusal_moves_to_the_next_code() {
        assert!(is_authcode_refusal("check authcode failed (400)"));
        assert!(!is_authcode_refusal("do_alarm_query invalid args (400)"));
        assert!(!is_authcode_refusal("cloud is not reachable: timed out"));
    }

    #[test]
    fn a_picture_name_cannot_walk_the_path() {
        assert!(object_name("003_DEV0001_261027203358.jpeg").is_ok());
        for bad in ["", "../etc/passwd", "a/b.jpeg", "a?b", "a#b", "a b"] {
            assert!(object_name(bad).is_err(), "{bad} was accepted");
        }
    }

    #[test]
    fn a_serial_is_a_plain_token() {
        assert!(plain_token("7f92970ddf5282743hrc"));
        assert!(!plain_token(""));
        assert!(!plain_token("a/b"));
        assert!(!plain_token("a b"));
    }

    #[test]
    fn a_bucket_is_a_plain_token() {
        assert_eq!(
            bucket_name(Some("OSS_oss-ap-id-pic-01"))
                .ok()
                .expect("bucket"),
            "OSS_oss-ap-id-pic-01"
        );
        assert!(bucket_name(None).is_err());
        assert!(bucket_name(Some("  ")).is_err());
        assert!(bucket_name(Some("a/b")).is_err());
    }

    fn live(key: &str) -> String {
        std::env::var(key).unwrap_or_default()
    }

    #[rocket::async_test]
    #[ignore = "talks to the vendor cloud"]
    async fn the_picture_route_returns_a_vendor_thumbnail() {
        use proview_core::cloud::CloudDevice;
        use proview_core::provider::VendorSession;
        use rocket::http::Cookie;

        let token = live("PROVIEW_LIVE_TOKEN");
        let serial = live("PROVIEW_LIVE_SERIAL");
        assert!(!token.is_empty(), "set PROVIEW_LIVE_TOKEN");
        assert!(!serial.is_empty(), "set PROVIEW_LIVE_SERIAL");

        let rig = rocket::custom(rocket::Config::figment().merge(("log_level", "critical")))
            .manage(CloudState::new())
            .manage(Store::new())
            .mount("/api", routes![picture]);
        let client = rocket::local::asynchronous::Client::tracked(rig)
            .await
            .expect("client");
        client
            .rocket()
            .state::<Store>()
            .expect("session store")
            .put(
                "live".to_string(),
                VendorSession {
                    account: "owner".to_string(),
                    token: Some(token),
                },
                std::time::Duration::from_secs(600),
            );
        let device: CloudDevice = serde_json::from_value(serde_json::json!({
            "uuid": serial,
            "pms": live("PROVIEW_LIVE_CODE"),
        }))
        .expect("device");
        client
            .rocket()
            .state::<CloudState>()
            .expect("cloud state")
            .keep_devices("live", &[device]);

        let target = format!(
            "/api/cloud/alarms/picture?sn={}&object={}&bucket={}",
            serial,
            live("PROVIEW_LIVE_OBJECT"),
            live("PROVIEW_LIVE_BUCKET")
        );
        let mut route_ms = Vec::new();
        let mut bytes = Vec::new();
        for _ in 0..5 {
            let clock = std::time::Instant::now();
            let reply = client
                .get(target.clone())
                .private_cookie(Cookie::new("pvw_session", "live"))
                .dispatch()
                .await;
            route_ms.push(clock.elapsed().as_millis() as u64);
            let status = reply.status();
            if status != Status::Ok {
                let body = reply.into_string().await.unwrap_or_default();
                panic!("picture route refused {status}: {body}");
            }
            bytes = reply.into_bytes().await.expect("picture bytes");
        }
        assert!(bytes.len() > 1000, "{} bytes", bytes.len());
        assert_eq!(&bytes[..2], &[0xff, 0xd8]);
        println!(
            "route picture bytes={} route_ms={route_ms:?} median={} max={}",
            bytes.len(),
            median(&route_ms),
            worst(&route_ms)
        );
    }

    fn median(values: &[u64]) -> u64 {
        let mut sorted = values.to_vec();
        sorted.sort_unstable();
        sorted[sorted.len() / 2]
    }

    fn worst(values: &[u64]) -> u64 {
        values.iter().copied().max().unwrap_or_default()
    }

    #[rocket::async_test]
    async fn the_picture_route_answers_with_the_object_bytes() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind vendor");
        let port = listener.local_addr().expect("vendor address").port();
        let vendor = std::thread::spawn(move || {
            let (mut sign, _) = listener.accept().expect("sign accept");
            let mut buffer = [0u8; 8192];
            let read = sign.read(&mut buffer).expect("sign read");
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            assert!(request.contains("MSG_DOWNLOAD_SIGN_REQ"), "{request}");
            assert!(request.contains("OSS_bucket"), "{request}");
            let payload = format!(
                concat!(
                    "{{\"CssCenter\":{{\"Body\":{{\"ObjInfo\":[{{\"Schema\":\"http\",",
                    "\"URL\":\"/003_DEV0001_261027214632.jpeg\",",
                    "\"ReqHeader\":{{\"Date\":\"Tue, 27 Oct 2026 16:09:33 GMT\",",
                    "\"Authorization\":\"OSS key:sig\"}},",
                    "\"Host\":\"127.0.0.1:{}\"}}],\"SerialNumber\":\"DEV0001\",\"Method\":\"GET\"}},",
                    "\"Header\":{{\"Version\":\"1.0\",\"MessageType\":\"MSG_DOWNLOAD_SIGN_RSP\",",
                    "\"ErrorNum\":\"200\",\"ErrorString\":\"Success OK\"}}}}}}"
                ),
                port
            );
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            sign.write_all(reply.as_bytes()).expect("sign write");
            drop(sign);

            let (mut object, _) = listener.accept().expect("object accept");
            let read = object.read(&mut buffer).expect("object read");
            let request = String::from_utf8_lossy(&buffer[..read]).to_lowercase();
            assert!(
                request.starts_with("get /003_dev0001_261027214632.jpeg "),
                "{request}"
            );
            assert!(request.contains("authorization: oss key:sig"), "{request}");
            let body = [0xffu8, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46];
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            object.write_all(head.as_bytes()).expect("object head");
            object.write_all(&body).expect("object body");
        });

        let client = seeded(
            proview_core::cloud::CssClient::new(format!("127.0.0.1:{port}")),
            "DEV0001",
        )
        .await;

        let reply = picture_of(
            &client,
            "DEV0001",
            "003_DEV0001_261027214632.jpeg",
            "OSS_bucket",
        )
        .await;
        let status = reply.status();
        if status != Status::Ok {
            let body = reply.into_string().await.unwrap_or_default();
            panic!("picture route refused {status}: {body}");
        }
        assert_eq!(reply.content_type(), Some(ContentType::JPEG));
        assert_eq!(
            reply.headers().get_one("Cache-Control"),
            Some("private, max-age=86400, immutable")
        );
        let bytes = reply.into_bytes().await.expect("picture bytes");
        vendor.join().expect("vendor thread");
        assert_eq!(bytes.len(), 8);
        assert_eq!(&bytes[..2], &[0xff, 0xd8]);
    }

    #[rocket::async_test]
    async fn the_picture_route_refuses_an_object_from_another_device() {
        let client = seeded(
            proview_core::cloud::CssClient::new("127.0.0.1:1"),
            "DEV0001",
        )
        .await;
        let reply = picture_of(
            &client,
            "DEV0001",
            "003_OTHER0001_261027214632.jpeg",
            "OSS_bucket",
        )
        .await;
        assert_eq!(reply.status(), Status::BadRequest);
        let body = reply.into_string().await.expect("body");
        assert!(body.contains("does not belong to this device"), "{body}");
    }

    #[rocket::async_test]
    async fn the_picture_route_reports_a_storage_refusal_as_upstream() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind vendor");
        let port = listener.local_addr().expect("vendor address").port();
        let vendor = std::thread::spawn(move || {
            let (mut sign, _) = listener.accept().expect("sign accept");
            let mut buffer = [0u8; 8192];
            let read = sign.read(&mut buffer).expect("sign read");
            assert!(String::from_utf8_lossy(&buffer[..read]).contains("MSG_DOWNLOAD_SIGN_REQ"));
            let payload = format!(
                concat!(
                    "{{\"CssCenter\":{{\"Body\":{{\"ObjInfo\":[{{\"Schema\":\"http\",",
                    "\"URL\":\"/003_DEV0001_261027214632.jpeg\",",
                    "\"ReqHeader\":{{\"Date\":\"Tue, 27 Oct 2026 16:09:33 GMT\",",
                    "\"Authorization\":\"OSS key:sig\"}},",
                    "\"Host\":\"127.0.0.1:{}\"}}],\"SerialNumber\":\"DEV0001\",\"Method\":\"GET\"}},",
                    "\"Header\":{{\"Version\":\"1.0\",\"MessageType\":\"MSG_DOWNLOAD_SIGN_RSP\",",
                    "\"ErrorNum\":\"200\",\"ErrorString\":\"Success OK\"}}}}}}"
                ),
                port
            );
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            sign.write_all(reply.as_bytes()).expect("sign write");
            drop(sign);

            let (mut object, _) = listener.accept().expect("object accept");
            let _ = object.read(&mut buffer);
            let body = concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>AccessDenied</Code>",
                "<Message>The bucket you access does not belong to you.</Message>",
                "<RequestId>6A1B2C3D4E5F60718293A4B5</RequestId><HostId>oss-ap-id-pic-01</HostId></Error>"
            );
            let head = format!(
                "HTTP/1.1 403 Forbidden\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            object.write_all(head.as_bytes()).expect("object head");
        });

        let client = seeded(
            proview_core::cloud::CssClient::new(format!("127.0.0.1:{port}")),
            "DEV0001",
        )
        .await;
        let reply = picture_of(
            &client,
            "DEV0001",
            "003_DEV0001_261027214632.jpeg",
            "OSS_bucket",
        )
        .await;
        assert_eq!(reply.status(), Status::BadGateway);
        let body = reply.into_string().await.expect("body");
        vendor.join().expect("vendor thread");
        assert!(body.len() < 300, "the vendor body was copied whole: {body}");
        assert!(body.contains("403"), "{body}");
    }

    async fn seeded(
        css: proview_core::cloud::CssClient,
        serial: &str,
    ) -> rocket::local::asynchronous::Client {
        use proview_core::provider::VendorSession;

        let rig = rocket::custom(
            rocket::Config::figment()
                .merge(("secret_key", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=="))
                .merge(("log_level", "critical")),
        )
        .manage(CloudState::with_css(css))
        .manage(Store::new())
        .mount("/api", routes![picture]);
        let client = rocket::local::asynchronous::Client::tracked(rig)
            .await
            .expect("client");
        client
            .rocket()
            .state::<Store>()
            .expect("session store")
            .put(
                "s1".to_string(),
                VendorSession {
                    account: "acc".to_string(),
                    token: Some("h.eyJhY2NvdW50IjoiYWNjMSJ9.s".to_string()),
                },
                std::time::Duration::from_secs(600),
            );
        let device: CloudDevice = serde_json::from_value(serde_json::json!({
            "uuid": serial,
            "nickname": "Gate",
            "pms": "aaaaaaaa-bbbbbbbbbbbbbbbbbbbbbbb"
        }))
        .expect("device");
        client
            .rocket()
            .state::<CloudState>()
            .expect("cloud state")
            .keep_devices("s1", &[device]);
        client
    }

    type Seen = std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>;

    fn alarm_center(replies: Vec<&'static str>) -> (u16, Seen) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind alarm centre");
        let port = listener.local_addr().expect("alarm address").port();
        let seen: Seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = seen.clone();
        std::thread::spawn(move || {
            for reply in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 8192];
                loop {
                    let read = stream.read(&mut chunk).unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    let text = String::from_utf8_lossy(&buffer).to_string();
                    if let Some(split) = text.find("\r\n\r\n") {
                        let length = text[..split]
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        if buffer.len() >= split + 4 + length {
                            let body = &text[split + 4..split + 4 + length];
                            record
                                .lock()
                                .expect("seen")
                                .push(serde_json::from_str(body).unwrap_or_default());
                            break;
                        }
                    }
                }
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.len(),
                    reply
                );
                let _ = stream.write_all(answer.as_bytes());
            }
        });
        (port, seen)
    }

    async fn rigged(port: u16) -> rocket::local::asynchronous::Client {
        rigged_with(
            port,
            serde_json::json!({ "uuid": "serial-one", "nickname": "Gate", "pms": "pms-code-one" }),
        )
        .await
    }

    async fn rigged_with(
        port: u16,
        record: serde_json::Value,
    ) -> rocket::local::asynchronous::Client {
        use proview_core::provider::VendorSession;

        let rig = rocket::custom(
            rocket::Config::figment()
                .merge(("secret_key", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=="))
                .merge(("log_level", "critical")),
        )
        .manage(
            CloudState::with_css(proview_core::cloud::CssClient::new("127.0.0.1:1"))
                .with_alarms(AlarmClient::new("127.0.0.1").with_port(port)),
        )
        .manage(Store::new())
        .mount("/api", routes![delete, calendar, subscription, list]);
        let client = rocket::local::asynchronous::Client::tracked(rig)
            .await
            .expect("client");
        client
            .rocket()
            .state::<Store>()
            .expect("session store")
            .put(
                "s1".to_string(),
                VendorSession {
                    account: "acc".to_string(),
                    token: Some("h.eyJhY2NvdW50IjoiYWNjMSJ9.s".to_string()),
                },
                std::time::Duration::from_secs(600),
            );
        let device: CloudDevice = serde_json::from_value(record).expect("device");
        client
            .rocket()
            .state::<CloudState>()
            .expect("cloud state")
            .keep_devices("s1", &[device]);
        client
    }

    const OK_EMPTY: &str =
        r#"{"AlarmCenter":{"Header":{"ErrorNum":"200","ErrorString":"Success OK"},"Body":{}}}"#;

    async fn post_delete(
        client: &rocket::local::asynchronous::Client,
        body: serde_json::Value,
    ) -> (Status, serde_json::Value) {
        let reply = client
            .post("/api/cloud/alarms/delete")
            .private_cookie(rocket::http::Cookie::new("pvw_session", "s1"))
            .header(ContentType::JSON)
            .body(body.to_string())
            .dispatch()
            .await;
        let status = reply.status();
        let text = reply.into_string().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or_default())
    }

    #[rocket::async_test]
    async fn deleting_selected_alarms_sends_their_ids_with_the_device_code_and_user() {
        let (port, seen) = alarm_center(vec![OK_EMPTY]);
        let client = rigged(port).await;
        let (status, body) = post_delete(
            &client,
            serde_json::json!({ "sn": "serial-one", "ids": ["261028205437", "261028205333"] }),
        )
        .await;
        assert_eq!(status, Status::Ok, "{body}");
        assert_eq!(body["deleted"], 2);
        assert_eq!(body["all"], false);
        let sent = seen.lock().expect("seen").clone();
        assert_eq!(sent.len(), 1);
        let center = &sent[0]["AlarmCenter"];
        assert_eq!(center["Header"]["MessageType"], "MSG_ALARM_DELETE_REQ");
        assert_eq!(center["Body"]["SerialNumber"], "serial-one");
        assert_eq!(center["Body"]["AuthCode"], "pms-code-one");
        assert_eq!(center["Body"]["UserId"], "acc1");
        assert_eq!(center["Body"]["DelType"], "MSG");
        assert_eq!(
            center["Body"]["AlarmId"],
            serde_json::json!([{ "id": "261028205437" }, { "id": "261028205333" }])
        );
    }

    #[rocket::async_test]
    async fn clearing_all_sends_no_id_list_and_an_empty_selection_never_reaches_the_cloud() {
        let (port, seen) = alarm_center(vec![OK_EMPTY]);
        let client = rigged(port).await;
        let (status, body) = post_delete(
            &client,
            serde_json::json!({ "sn": "serial-one", "ids": [] }),
        )
        .await;
        assert_eq!(status, Status::BadRequest, "{body}");
        assert!(seen.lock().expect("seen").is_empty());
        let (status, body) = post_delete(
            &client,
            serde_json::json!({ "sn": "serial-one", "all": true }),
        )
        .await;
        assert_eq!(status, Status::Ok, "{body}");
        assert_eq!(body["all"], true);
        let sent = seen.lock().expect("seen").clone();
        assert_eq!(sent.len(), 1);
        assert!(sent[0]["AlarmCenter"]["Body"].get("AlarmId").is_none());
    }

    fn shared_record(permissions: &str) -> serde_json::Value {
        serde_json::json!({
            "uuid": "serial-one",
            "nickname": "Gate",
            "pms": "pms-code-one",
            "shareId": "share-7",
            "deviceOwnerId": "owner-one",
            "devPermissions": permissions
        })
    }

    #[rocket::async_test]
    async fn a_camera_shared_without_the_delete_right_refuses_delete_and_clear_all() {
        let (port, seen) = alarm_center(vec![OK_EMPTY, OK_EMPTY]);
        let client = rigged_with(port, shared_record("DP_PTZ,DP_Intercom,DP_AlarmPush")).await;
        for body in [
            serde_json::json!({ "sn": "serial-one", "ids": ["261028205437"] }),
            serde_json::json!({ "sn": "serial-one", "all": true }),
        ] {
            let (status, reply) = post_delete(&client, body).await;
            assert_eq!(status, Status::Forbidden, "{reply}");
            assert!(
                reply
                    .to_string()
                    .contains("without the right to delete alarm messages"),
                "{reply}"
            );
        }
        assert!(seen.lock().expect("seen").is_empty());
    }

    #[rocket::async_test]
    async fn a_camera_shared_with_the_delete_right_and_an_owned_one_may_delete() {
        let (port, seen) = alarm_center(vec![OK_EMPTY]);
        let client = rigged_with(port, shared_record("DP_PTZ,DP_DeleteAlarmInfo")).await;
        let (status, reply) = post_delete(
            &client,
            serde_json::json!({ "sn": "serial-one", "all": true }),
        )
        .await;
        assert_eq!(status, Status::Ok, "{reply}");
        assert_eq!(seen.lock().expect("seen").len(), 1);
        let listed = serde_json::json!({ "uuid": "x", "shareId": "s", "permissions": [
            { "label": "DP_DeleteAlarmInfo", "enabled": true }
        ]});
        assert!(delete_refusal(&serde_json::from_value(listed).expect("device")).is_none());
        let disabled = serde_json::json!({ "uuid": "x", "shareId": "s", "permissions": [
            { "label": "DP_DeleteAlarmInfo", "enabled": false }
        ]});
        assert!(delete_refusal(&serde_json::from_value(disabled).expect("device")).is_some());
        let owned = serde_json::json!({ "uuid": "x", "mAccount": true });
        assert!(delete_refusal(&serde_json::from_value(owned).expect("device")).is_none());
    }

    #[rocket::async_test]
    async fn a_refused_delete_is_reported_with_the_vendor_reason() {
        let (port, _) = alarm_center(vec![
            r#"{"AlarmCenter":{"Header":{"ErrorNum":"400","ErrorString":"invalid parameter"}}}"#,
        ]);
        let client = rigged(port).await;
        let (status, body) = post_delete(
            &client,
            serde_json::json!({ "sn": "serial-one", "ids": ["1"] }),
        )
        .await;
        assert_eq!(status, Status::BadRequest, "{body}");
        assert!(
            body.to_string().contains("invalid parameter (400)"),
            "{body}"
        );
    }

    #[rocket::async_test]
    async fn the_calendar_route_lists_days_with_alarms() {
        let (port, seen) = alarm_center(vec![
            r#"{"AlarmCenter":{"Header":{"ErrorNum":"200"},"Body":{"Date":[{"Time":"2026-10-28","Count":"14"},{"Time":"2026-10-27","Count":"52"}]}}}"#,
        ]);
        let client = rigged(port).await;
        let reply = client
            .get("/api/cloud/alarms/calendar?sn=serial-one&month=2026-10")
            .private_cookie(rocket::http::Cookie::new("pvw_session", "s1"))
            .dispatch()
            .await;
        assert_eq!(reply.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&reply.into_string().await.unwrap_or_default()).expect("json");
        assert_eq!(
            body["days"],
            serde_json::json!([{ "day": "2026-10-27", "count": 52 }, { "day": "2026-10-28", "count": 14 }])
        );
        let sent = seen.lock().expect("seen").clone();
        assert_eq!(
            sent[0]["AlarmCenter"]["Header"]["MessageType"],
            "MSG_VIDEO_NODE_QUERY_REQ"
        );
        assert_eq!(sent[0]["AlarmCenter"]["Body"]["Date"], "2026-10");
        assert_eq!(sent[0]["AlarmCenter"]["Body"]["Type"], "MSG");
        let bad = client
            .get("/api/cloud/alarms/calendar?sn=serial-one&month=2026-13")
            .private_cookie(rocket::http::Cookie::new("pvw_session", "s1"))
            .dispatch()
            .await;
        assert_eq!(bad.status(), Status::BadRequest);
    }

    #[rocket::async_test]
    async fn the_subscription_route_reports_the_tokens_the_centre_pushes_to() {
        let (port, seen) = alarm_center(vec![
            r#"{"AlarmCenter":{"Header":{"ErrorNum":"200"},"Body":[{"Token":["phone-a"]}]}}"#,
            r#"{"AlarmCenter":{"Header":{"ErrorNum":"200"},"Body":[{"Token":["other-phone-token","phone-b"]}]}}"#,
        ]);
        let client = rigged(port).await;
        let reply = client
            .get("/api/cloud/alarms/subscription?sn=serial-one&tokens=phone-a")
            .private_cookie(rocket::http::Cookie::new("pvw_session", "s1"))
            .dispatch()
            .await;
        assert_eq!(reply.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&reply.into_string().await.unwrap_or_default()).expect("json");
        assert_eq!(body["subscribed"], true);
        assert_eq!(body["count"], 1);
        assert!(body.get("tokens").is_none(), "{body}");
        let bare = client
            .get("/api/cloud/alarms/subscription?sn=serial-one")
            .private_cookie(rocket::http::Cookie::new("pvw_session", "s1"))
            .dispatch()
            .await;
        let text = bare.into_string().await.unwrap_or_default();
        assert!(!text.contains("other-phone-token"), "{text}");
        let bare: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(bare["count"], 2);
        assert_eq!(bare["subscribed"], false);
        let sent = seen.lock().expect("seen").clone();
        assert_eq!(
            sent[0]["AlarmCenter"]["Body"]["Token"],
            serde_json::json!(["phone-a"])
        );
        assert_eq!(sent[0]["AlarmCenter"]["Body"]["SubType"], "ALL");
    }

    #[rocket::async_test]
    async fn an_event_filter_reaches_the_centre_and_an_unknown_one_does_not() {
        let (port, seen) = alarm_center(vec![
            r#"{"AlarmCenter":{"Header":{"ErrorNum":"200"},"Body":{"AlarmTotal":0,"AlarmArray":[],"IsFinished":"1"}}}"#,
        ]);
        let client = rigged(port).await;
        let reply = client
            .get("/api/cloud/alarms?sn=serial-one&event=VideoMotion")
            .private_cookie(rocket::http::Cookie::new("pvw_session", "s1"))
            .dispatch()
            .await;
        assert_eq!(reply.status(), Status::Ok);
        let refused = client
            .get("/api/cloud/alarms?sn=serial-one&event=Anything")
            .private_cookie(rocket::http::Cookie::new("pvw_session", "s1"))
            .dispatch()
            .await;
        assert_eq!(refused.status(), Status::BadRequest);
        let sent = seen.lock().expect("seen").clone();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["AlarmCenter"]["Body"]["AlarmEvent"], "VideoMotion");
        assert_eq!(
            sent[0]["AlarmCenter"]["Header"]["MessageType"],
            "MSG_ALARM_QUERY_BY_TIME_REQ"
        );
    }

    #[rocket::async_test]
    async fn a_silent_alarm_centre_turns_into_a_504_naming_the_stage() {
        let started = std::time::Instant::now();
        let outcome: Result<(), Failure> = within_window(
            "delete",
            std::time::Duration::from_millis(80),
            std::future::pending(),
        )
        .await;
        let Err((status, body)) = outcome else {
            panic!("a silent future must hit the deadline");
        };
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert_eq!(status, Status::GatewayTimeout);
        assert!(
            body.message.contains("delete request within 80 ms"),
            "{}",
            body.message
        );
    }

    async fn picture_of<'a>(
        client: &'a rocket::local::asynchronous::Client,
        serial: &str,
        object: &str,
        bucket: &str,
    ) -> rocket::local::asynchronous::LocalResponse<'a> {
        client
            .get(format!(
                "/api/cloud/alarms/picture?sn={serial}&object={object}&bucket={bucket}"
            ))
            .private_cookie(rocket::http::Cookie::new("pvw_session", "s1"))
            .dispatch()
            .await
    }
}
