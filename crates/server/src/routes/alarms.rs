use proview_core::cloud::{AlarmEntry, AlarmPage, AlarmQuery, CloudDevice, CloudError};
use proview_core::session::Store;
use rocket::http::{ContentType, CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;

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
}

pub fn routes() -> Vec<rocket::route::Route> {
    routes![list, picture]
}

pub fn files() -> Vec<rocket::route::Route> {
    routes![alarms_js]
}

#[get("/alarms.js")]
fn alarms_js() -> rocket::response::content::RawJavaScript<&'static str> {
    rocket::response::content::RawJavaScript(include_str!("../../../../web/alarms.js"))
}

#[get("/cloud/alarms?<sn>&<start>&<end>&<limit>")]
pub async fn list(
    sn: String,
    start: Option<String>,
    end: Option<String>,
    limit: Option<u32>,
    state: &State<CloudState>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<AlarmList>, Failure> {
    let session_id = session_id(cookies)?;
    let token = cloud_token(store, &session_id)?;
    let device = lookup(state, &session_id, &token, sn.trim()).await?;
    let page = query(state, &device, window(start, end), limit).await?;
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
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<(ContentType, Vec<u8>), Failure> {
    let session_id = session_id(cookies)?;
    let token = cloud_token(store, &session_id)?;
    let device = lookup(state, &session_id, &token, sn.trim()).await?;
    let name = object_name(&object)?;
    let picture = state
        .alarms()
        .picture(&device.uuid, &auth_code(&device), &name)
        .await
        .map_err(|error| vendor_failure(&error))?;
    Ok((ContentType::JPEG, picture))
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
) -> Result<AlarmPage, Failure> {
    let codes = auth_codes(device);
    if codes.is_empty() {
        return Err(fail(
            Status::ServiceUnavailable,
            "this device has no cloud auth code, so the alarm centre cannot answer for it",
        ));
    }
    let (start, end) = window;
    let number = limit
        .unwrap_or(proview_core::cloud::alarm::PAGE_LIMIT)
        .clamp(1, proview_core::cloud::alarm::PAGE_LIMIT);
    let client = state.alarms();
    let mut last = CloudError::Rejected("the alarm centre refused every auth code".to_string());
    for code in codes {
        let request = AlarmQuery {
            serial: device.uuid.clone(),
            auth_code: code,
            channel: 0,
            start: start.clone(),
            end: end.clone(),
            offset: 0,
            number,
        };
        match client.list(&request).await {
            Ok(page) => return Ok(page),
            Err(CloudError::Rejected(reason)) if is_authcode_refusal(&reason) => {
                last = CloudError::Rejected(reason);
            }
            Err(error) => return Err(vendor_failure(&error)),
        }
    }
    Err(vendor_failure(&last))
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
}
