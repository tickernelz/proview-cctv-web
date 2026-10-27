use std::io::Cursor;

use proview_core::cloud::{
    claims, AlarmEntry, AlarmPage, AlarmQuery, CloudDevice, CloudError, SignRequest,
};
use proview_core::session::Store;
use rocket::http::{ContentType, CookieJar, Status};
use rocket::request::Request;
use rocket::response::{Responder, Response};
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
            "pms": "aaaaaaaa-4a1c2f27d01793117365801"
        }))
        .expect("device");
        client
            .rocket()
            .state::<CloudState>()
            .expect("cloud state")
            .keep_devices("s1", &[device]);
        client
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
