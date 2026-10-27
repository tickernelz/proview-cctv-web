use proview_core::cloud::dss::{cloud_recordings, RecordingPage, RecordingQuery};
use proview_core::config::Config;
use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;

use super::{fail, restore, session_id, Failure};
use crate::config::ModeStore;

pub fn routes() -> Vec<rocket::route::Route> {
    routes![recordings]
}

#[get("/recordings?<device>&<channel>&<day>")]
async fn recordings(
    device: Option<String>,
    channel: Option<u32>,
    day: Option<String>,
    _cfg: &State<Config>,
    store: &State<Store>,
    _modes: &State<ModeStore>,
    cookies: &CookieJar<'_>,
) -> Result<Json<RecordingPage>, Failure> {
    let id = session_id(cookies)?;
    let session = restore(store, &id)?;
    let access = session.token.clone().ok_or_else(|| {
        fail(
            Status::NotImplemented,
            "this session has no cloud token; LAN recordings need the DVRIP file search, which the camera has not answered yet",
        )
    })?;
    let day = day.unwrap_or_else(today);
    let account = proview_core::cloud::claims(&access)
        .and_then(|claims| claims.account)
        .unwrap_or_else(|| session.account.clone());
    let request = RecordingQuery {
        access: &access,
        account: &account,
        sn: device.as_deref().unwrap_or_default(),
        channel: channel.unwrap_or(1),
        day: &day,
    };
    cloud_recordings(request)
        .await
        .map(Json)
        .map_err(|error| fail(status_of(&error), error.to_string()))
}

fn status_of(error: &proview_core::cloud::CloudError) -> Status {
    match error {
        proview_core::cloud::CloudError::Credentials => Status::Unauthorized,
        proview_core::cloud::CloudError::Expired => Status::Unauthorized,
        proview_core::cloud::CloudError::Unreachable(_) => Status::ServiceUnavailable,
        proview_core::cloud::CloudError::Rejected(_) => Status::BadGateway,
        proview_core::cloud::CloudError::Malformed(_) => Status::BadGateway,
    }
}

fn today() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|span| span.as_secs() as i64)
        .unwrap_or_default();
    let shifted = now + 7 * 3_600;
    let days = shifted.div_euclid(86_400);
    let seconds = shifted.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02}:{seconds:02}",
        hours = seconds / 3_600,
        minutes = seconds % 3_600 / 60,
        seconds = seconds % 60
    )
    .split(' ')
    .next()
    .unwrap_or_default()
    .to_string()
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { year + 1 } else { year }, month, day)
}
