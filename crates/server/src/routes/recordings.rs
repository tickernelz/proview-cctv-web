use proview_core::cloud::cloud_clip::window;
use proview_core::cloud::dss::{cloud_recordings, RecordingError, RecordingPage, RecordingQuery};
use proview_core::cloud::CloudError;
use proview_core::config::Config;
use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;

use super::{fail, restore, session_id, Failure};

pub fn routes() -> Vec<rocket::route::Route> {
    routes![recordings]
}

#[get("/recordings?<device>&<channel>&<day>")]
async fn recordings(
    device: Option<String>,
    channel: Option<u32>,
    day: Option<String>,
    cfg: &State<Config>,
    store: &State<Store>,
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
    let day = day.unwrap_or_default();
    if !is_day(&day) {
        return Err(fail(
            Status::BadRequest,
            "day must be a local date in YYYY-MM-DD form",
        ));
    }
    let account = proview_core::cloud::claims(&access)
        .and_then(|claims| claims.account)
        .unwrap_or_else(|| session.account.clone());
    let request = RecordingQuery {
        access: &access,
        account: &account,
        sn: device.as_deref().unwrap_or_default(),
        channel: channel.unwrap_or_default(),
        day: &day,
    };
    let client = super::cloud_clips::envelope_for(cfg);
    let limit = window();
    match tokio::time::timeout(limit, cloud_recordings(&client, request)).await {
        Ok(reply) => reply
            .map(Json)
            .map_err(|error| fail(status_of(&error), error.to_string())),
        Err(_) => Err(fail(
            Status::GatewayTimeout,
            format!(
                "the cloud clip list stage did not answer within {} ms",
                limit.as_millis()
            ),
        )),
    }
}

fn status_of(error: &RecordingError) -> Status {
    match error {
        RecordingError::UnknownDevice(_) => Status::NotFound,
        RecordingError::MissingAuth(_) => Status::BadGateway,
        RecordingError::Cloud(error) => match error {
            CloudError::Credentials | CloudError::Expired => Status::Unauthorized,
            CloudError::Unreachable(_) => Status::ServiceUnavailable,
            CloudError::Malformed(_) | CloudError::Rejected(_) => Status::BadGateway,
        },
    }
}

fn is_day(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}
