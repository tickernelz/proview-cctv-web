use std::collections::HashMap;
use std::io::Cursor;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use proview_core::cloud::cloud_clip::{
    decide, fetch_clip, hls_body, owned_object, plan_from_caps, plan_from_css, window, ClipError,
    Evidence, Plan, PlanState,
};
use proview_core::cloud::{
    claims, CapsClient, CloudDevice, CloudError, EnvelopeClient, Identity, SignRequest,
};
use proview_core::config::Config;
use proview_core::session::Store;
use rocket::http::{ContentType, CookieJar, Status};
use rocket::request::Request;
use rocket::response::{Responder, Response};
use rocket::serde::json::Json;
use rocket::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::alarms::{now_seconds, stamp_at, AUTH_KEYS};
use super::cloud::CloudState;
use crate::routes::{fail, restore, session_id, Failure};

pub const REMUX_WINDOW: Duration = Duration::from_secs(120);

pub fn routes() -> Vec<rocket::route::Route> {
    routes![state, download, play]
}

pub(crate) fn envelope_for(cfg: &Config) -> Arc<EnvelopeClient> {
    static POOL: OnceLock<Mutex<HashMap<String, Arc<EnvelopeClient>>>> = OnceLock::new();
    let pool = POOL.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = pool.lock().unwrap_or_else(|error| error.into_inner());
    guard
        .entry(cfg.cloud_base.clone())
        .or_insert_with(|| {
            Arc::new(EnvelopeClient::with_base(
                Identity::resolve(),
                cfg.cloud_base.clone(),
            ))
        })
        .clone()
}

fn caps_client() -> &'static CapsClient {
    static CAPS: OnceLock<CapsClient> = OnceLock::new();
    CAPS.get_or_init(CapsClient::from_env)
}

struct Owner {
    device: CloudDevice,
    user_id: String,
}

impl Owner {
    fn code(&self, preferred: &str) -> String {
        std::iter::once(preferred)
            .chain(AUTH_KEYS)
            .filter_map(|key| self.device.extra.get(key).and_then(|value| value.as_str()))
            .map(str::trim)
            .find(|code| !code.is_empty())
            .unwrap_or_default()
            .to_string()
    }
}

fn plain_serial(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || matches!(letter, '_' | '-'))
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
            "the cloud clip carries no storage bucket",
        ));
    }
    Ok(name.to_string())
}

fn vendor_status(error: &CloudError) -> Status {
    match error {
        CloudError::Credentials | CloudError::Expired => Status::Unauthorized,
        CloudError::Unreachable(_) => Status::ServiceUnavailable,
        CloudError::Rejected(_) | CloudError::Malformed(_) => Status::BadGateway,
    }
}

async fn owner(
    cfg: &Config,
    cloud: &CloudState,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
    serial: &str,
) -> Result<Owner, Failure> {
    let id = session_id(cookies)?;
    let session = restore(store, &id)?;
    let token = session
        .token
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            fail(
                Status::Unauthorized,
                "cloud clips live on the vendor cloud; sign in with the cloud account, not the camera on this network",
            )
        })?;
    let serial = serial.trim();
    if !plain_serial(serial) {
        return Err(fail(Status::BadRequest, "a device serial is required"));
    }
    let user_id = claims(&token)
        .and_then(|claims| claims.account)
        .unwrap_or_default();
    let find = |devices: &[CloudDevice]| {
        devices
            .iter()
            .find(|device| device.uuid.eq_ignore_ascii_case(serial))
            .cloned()
    };
    if let Some(device) = cloud.cached_devices(&id).as_deref().and_then(find) {
        return Ok(Owner { device, user_id });
    }
    let limit = window();
    let devices = match tokio::time::timeout(limit, envelope_for(cfg).device_list(&token)).await {
        Ok(Ok(devices)) => devices,
        Ok(Err(error)) => return Err(fail(vendor_status(&error), error.to_string())),
        Err(_) => {
            return Err(fail(
                Status::GatewayTimeout,
                format!(
                    "the cloud clip device list stage did not answer within {} ms",
                    limit.as_millis()
                ),
            ))
        }
    };
    cloud.keep_devices(&id, &devices);
    find(&devices)
        .map(|device| Owner { device, user_id })
        .ok_or_else(|| {
            fail(
                Status::NotFound,
                format!("{serial} is not a device on this account"),
            )
        })
}

fn day_or_today(day: Option<String>) -> Result<String, Failure> {
    let day = day
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| stamp_at(now_seconds())[..10].to_string());
    let bytes = day.as_bytes();
    let shaped = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
    if !shaped {
        return Err(fail(
            Status::BadRequest,
            "day must be a local date in YYYY-MM-DD form",
        ));
    }
    Ok(day)
}

fn silent(source: &'static str, detail: String) -> (Plan, Evidence) {
    (
        Plan::Unknown,
        Evidence {
            source,
            code: "none".to_string(),
            detail,
        },
    )
}

#[get("/cloud/clips/state?<device>&<channel>&<day>")]
async fn state(
    device: String,
    channel: Option<u32>,
    day: Option<String>,
    cfg: &State<Config>,
    cloud: &State<CloudState>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<PlanState>, Failure> {
    let channel = channel.unwrap_or_default();
    let day = day_or_today(day)?;
    let owner = owner(cfg, cloud, store, cookies, &device).await?;
    let serial = owner.device.uuid.clone();
    let limit = window();
    let caps_ask = tokio::time::timeout(limit, caps_client().req_caps(&serial, &["xmc.css"]));
    let body = hls_body(&serial, &owner.code("css"), &owner.user_id, &day, channel);
    let css_ask = tokio::time::timeout(limit, cloud.css().center(&body));
    let (caps, css) = tokio::join!(caps_ask, css_ask);
    let millis = limit.as_millis();
    let caps = match caps {
        Ok(Ok(reply)) => plan_from_caps(&reply.caps, channel, now_seconds()),
        Ok(Err(error)) => silent("caps", error.to_string()),
        Err(_) => silent("caps", format!("caps did not answer within {millis} ms")),
    };
    let css = match css {
        Ok(Ok(reply)) => plan_from_css(&reply),
        Ok(Err(error)) => silent("css", error.to_string()),
        Err(_) => silent("css", format!("css did not answer within {millis} ms")),
    };
    let timed_out = |evidence: &Evidence| evidence.detail.contains("did not answer within");
    if timed_out(&caps.1) && timed_out(&css.1) {
        return Err(fail(
            Status::GatewayTimeout,
            format!("the cloud clip plan stage (caps and css) did not answer within {millis} ms"),
        ));
    }
    Ok(Json(decide(&serial, Some(caps), Some(css))))
}

pub struct Clip {
    bytes: Vec<u8>,
    kind: ContentType,
    attachment: Option<String>,
}

impl<'r> Responder<'r, 'r> for Clip {
    fn respond_to(self, _: &'r Request<'_>) -> rocket::response::Result<'r> {
        let mut response = Response::build();
        response
            .header(self.kind)
            .raw_header("Cache-Control", "private, no-store");
        if let Some(name) = self.attachment {
            response.raw_header(
                "Content-Disposition",
                format!("attachment; filename=\"{name}\""),
            );
        }
        response
            .sized_body(self.bytes.len(), Cursor::new(self.bytes))
            .ok()
    }
}

fn clip_failure(error: &ClipError) -> Failure {
    tracing::warn!(%error, "cloud clip fetch failed");
    let status = match error {
        ClipError::Timeout { .. } => Status::GatewayTimeout,
        ClipError::Cloud { error, .. } => vendor_status(error),
        ClipError::Playlist(_) | ClipError::TooLarge => Status::BadGateway,
    };
    fail(status, error.to_string())
}

async fn clip_bytes(
    cfg: &Config,
    cloud: &CloudState,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
    device: &str,
    object: &str,
    bucket: Option<&str>,
) -> Result<(String, Vec<u8>), Failure> {
    let owner = owner(cfg, cloud, store, cookies, device).await?;
    let object = object.trim();
    if !owned_object(object, &owner.device.uuid) {
        return Err(fail(
            Status::BadRequest,
            "that clip does not belong to this device",
        ));
    }
    let request = SignRequest {
        serial: owner.device.uuid.clone(),
        auth_code: owner.code("css"),
        user_id: owner.user_id.clone(),
        object: object.to_string(),
        bucket: bucket_name(bucket)?,
    };
    let bytes = fetch_clip(cloud.css(), &request, window())
        .await
        .map_err(|error| clip_failure(&error))?;
    let stem = object
        .rsplit('/')
        .next()
        .unwrap_or(object)
        .trim_end_matches(".m3u8");
    Ok((format!("{}-{stem}", owner.device.uuid), bytes))
}

#[get("/cloud/clips/download?<device>&<object>&<bucket>")]
async fn download(
    device: String,
    object: String,
    bucket: Option<String>,
    cfg: &State<Config>,
    cloud: &State<CloudState>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Clip, Failure> {
    let (name, bytes) = clip_bytes(
        cfg,
        cloud,
        store,
        cookies,
        &device,
        &object,
        bucket.as_deref(),
    )
    .await?;
    Ok(Clip {
        bytes,
        kind: ContentType::new("video", "mp2t"),
        attachment: Some(format!("{name}.ts")),
    })
}

#[get("/cloud/clips/play?<device>&<object>&<bucket>")]
async fn play(
    device: String,
    object: String,
    bucket: Option<String>,
    cfg: &State<Config>,
    cloud: &State<CloudState>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Clip, Failure> {
    let (_, bytes) = clip_bytes(
        cfg,
        cloud,
        store,
        cookies,
        &device,
        &object,
        bucket.as_deref(),
    )
    .await?;
    let bytes = remux(bytes, REMUX_WINDOW).await?;
    Ok(Clip {
        bytes,
        kind: ContentType::MP4,
        attachment: None,
    })
}

fn ffmpeg_binary() -> String {
    std::env::var("PROVIEW_FFMPEG")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "ffmpeg".to_string())
}

fn remuxer(binary: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(binary);
    proview_core::process::hide_console(command.as_std_mut());
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "mpegts",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-profile:v",
            "main",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "64k",
            "-f",
            "mp4",
            "-movflags",
            "+empty_moov+default_base_moof+frag_keyframe",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

async fn remux(ts: Vec<u8>, limit: Duration) -> Result<Vec<u8>, Failure> {
    remux_with(&ffmpeg_binary(), ts, limit).await
}

async fn remux_with(binary: &str, ts: Vec<u8>, limit: Duration) -> Result<Vec<u8>, Failure> {
    let mut child = remuxer(binary).spawn().map_err(|error| {
        fail(
            Status::NotImplemented,
            format!(
                "ffmpeg did not start, so the clip cannot play here; download it instead: {error}"
            ),
        )
    })?;
    let mut stdin = child.stdin.take();
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let work = async move {
        let feed = async {
            if let Some(mut pipe) = stdin.take() {
                let _ = pipe.write_all(&ts).await;
                let _ = pipe.shutdown().await;
            }
        };
        let read = async {
            let mut out = Vec::new();
            if let Some(pipe) = stdout.as_mut() {
                let _ = pipe.read_to_end(&mut out).await;
            }
            out
        };
        let errors = async {
            let mut text = String::new();
            if let Some(pipe) = stderr.as_mut() {
                let _ = pipe.read_to_string(&mut text).await;
            }
            text
        };
        let ((), out, text) = tokio::join!(feed, read, errors);
        let status = child.wait().await;
        (status, out, text)
    };
    match tokio::time::timeout(limit, work).await {
        Ok((Ok(status), out, _)) if status.success() && !out.is_empty() => Ok(out),
        Ok((_, _, text)) => Err(fail(
            Status::BadGateway,
            format!(
                "ffmpeg could not turn the clip into MP4: {}",
                text.trim().chars().take(200).collect::<String>()
            ),
        )),
        Err(_) => Err(fail(
            Status::GatewayTimeout,
            format!(
                "the cloud clip remux stage did not finish within {} ms",
                limit.as_millis()
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_and_serials_refuse_anything_but_plain_names() {
        assert!(bucket_name(Some("OSS_vid-01")).is_ok());
        assert!(bucket_name(Some("")).is_err());
        assert!(bucket_name(Some("a/b")).is_err());
        assert!(bucket_name(None).is_err());
        assert!(plain_serial("SN-0001"));
        assert!(!plain_serial("SN 0001"));
        assert!(!plain_serial(""));
    }

    #[test]
    fn days_default_to_today_and_refuse_other_shapes() {
        assert_eq!(
            day_or_today(Some("2026-10-29".into())).ok().as_deref(),
            Some("2026-10-29")
        );
        assert_eq!(day_or_today(None).ok().map(|day| day.len()), Some(10));
        assert!(day_or_today(Some("29/10/2026".into())).is_err());
    }

    #[test]
    fn auth_code_prefers_css_then_falls_back_in_app_order() {
        let device: CloudDevice = serde_json::from_value(serde_json::json!({
            "uuid": "SN-0001", "pms": "pms-code", "css": "css-code"
        }))
        .unwrap();
        let owner = Owner {
            device,
            user_id: String::new(),
        };
        assert_eq!(owner.code("css"), "css-code");
        assert_eq!(owner.code("dss"), "pms-code");
    }

    #[cfg(unix)]
    fn script(name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("pvw-remux-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ffmpeg");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh
{body}
"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_remuxer_that_never_finishes_is_a_remux_timeout() {
        let path = script("hang", "sleep 5");
        let started = std::time::Instant::now();
        let error = remux_with(
            path.to_str().unwrap(),
            vec![0x47; 188],
            Duration::from_millis(300),
        )
        .await
        .map(|_| ())
        .expect_err("a hung remuxer fails");
        assert_eq!(error.0, Status::GatewayTimeout);
        assert!(
            error.1.message.contains("remux stage"),
            "{}",
            error.1.message
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_remuxer_gets_the_whole_stream_and_its_output_is_the_clip() {
        let path = script("echo", "printf MP4:; cat");
        let out = remux_with(
            path.to_str().unwrap(),
            b"TSDATA".to_vec(),
            Duration::from_secs(5),
        )
        .await
        .map_err(|failure| failure.1.message.clone())
        .expect("the remuxer output");
        assert_eq!(out, b"MP4:TSDATA");
        let failing = script("fail", "cat >/dev/null; echo broken >&2; exit 1");
        let error = remux_with(
            failing.to_str().unwrap(),
            b"TS".to_vec(),
            Duration::from_secs(5),
        )
        .await
        .map(|_| ())
        .expect_err("a failing remuxer fails");
        assert_eq!(error.0, Status::BadGateway);
        assert!(error.1.message.contains("broken"), "{}", error.1.message);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let _ = std::fs::remove_dir_all(failing.parent().unwrap());
    }
}
