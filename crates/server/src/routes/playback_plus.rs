use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use proview_core::dvrip::playback::{
    access_unit, codec_of, MediaReader, PlaybackError, PlaybackFile, RecordKind, Session,
    PLAYBACK_DATA,
};
use proview_core::dvrip::playback_plus::{
    is_jpeg, jpeg_data_url as data_url, picture_inside, RecordFilter,
};
use proview_core::media::Codec;
use proview_core::provider::cloud::CloudProvider;
use proview_core::provider::VendorSession;
use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Notify;

use crate::routes::{fail, restore, session_id, status_for, Failure};

const WINDOW_ENV: &str = "PROVIEW_PLAYBACK_WINDOW_MS";
const DEFAULT_WINDOW: Duration = Duration::from_secs(20);
const PICTURE_WAIT: Duration = Duration::from_secs(6);
const KEYFRAME_WAIT: Duration = Duration::from_secs(8);
const DECODE_WAIT: Duration = Duration::from_secs(10);
const MAX_THUMBS: usize = 6;
pub const BATCH_DEADLINE: Duration = Duration::from_secs(60);
const KEYFRAME_RESERVE: Duration = Duration::from_secs(12);
const CACHE_LIMIT: usize = 1024;
pub const PACE_LEAD_SECONDS: f64 = 20.0;

pub fn routes() -> Vec<rocket::route::Route> {
    routes![calendar, thumbnails]
}

pub fn window() -> Duration {
    match std::env::var(WINDOW_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        Some(ms) if ms > 0 => Duration::from_millis(ms),
        _ => DEFAULT_WINDOW,
    }
}

pub fn too_slow(stage: &str, wait: Duration) -> Failure {
    fail(
        Status::GatewayTimeout,
        format!(
            "playback stage={stage}: the device did not answer inside {:.1} s",
            wait.as_secs_f64()
        ),
    )
}

pub fn valid_device(device: &str) -> Result<(), Failure> {
    if device.trim().is_empty() || device.contains('/') || device.contains("..") {
        return Err(fail(Status::BadRequest, "invalid device id"));
    }
    Ok(())
}

pub async fn open_tunnel(
    cloud: &CloudProvider,
    vendor: &VendorSession,
    serial: &str,
    wait: Duration,
) -> Result<Session, Failure> {
    match tokio::time::timeout(wait, cloud.open_playback(vendor, serial)).await {
        Ok(Ok((session, _))) => Ok(session),
        Ok(Err(error)) => Err(fail(status_for(&error), error.to_string())),
        Err(_) => Err(too_slow("tunnel", wait)),
    }
}

pub async fn within<T>(
    stage: &str,
    wait: Duration,
    work: impl std::future::Future<Output = Result<T, PlaybackError>>,
) -> Result<T, Failure> {
    match tokio::time::timeout(wait, work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(playback_failure(stage, error)),
        Err(_) => Err(too_slow(stage, wait)),
    }
}

pub fn playback_failure(stage: &str, error: PlaybackError) -> Failure {
    let status = match error {
        PlaybackError::NoReply(_) => Status::GatewayTimeout,
        _ => Status::BadGateway,
    };
    fail(status, format!("playback stage={stage}: {error}"))
}

#[derive(Serialize)]
pub struct CalendarBody {
    pub ok: bool,
    pub device: String,
    pub year: i32,
    pub month: u32,
    pub days: Vec<u32>,
    pub elapsed_ms: u64,
}

#[get("/playback/calendar?<device>&<year>&<month>")]
pub async fn calendar(
    device: String,
    year: i32,
    month: u32,
    cloud: &State<Arc<CloudProvider>>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<CalendarBody>, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    valid_device(&device)?;
    if !(2000..=2063).contains(&year) || !(1..=12).contains(&month) {
        return Err(fail(
            Status::BadRequest,
            "year must be 2000-2063 and month 1-12",
        ));
    }
    let started = Instant::now();
    let wait = window();
    let mut session = open_tunnel(cloud.inner(), &vendor, &device, wait).await?;
    let days = within("calendar", wait, session.calendar(year, month)).await?;
    let _ = tokio::time::timeout(Duration::from_secs(2), session.logout()).await;
    Ok(Json(CalendarBody {
        ok: true,
        device,
        year,
        month,
        days,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

#[derive(Deserialize)]
pub struct ThumbFile {
    pub name: String,
    pub begin: String,
    pub end: String,
}

#[derive(Deserialize)]
pub struct ThumbAsk {
    pub device: String,
    pub channel: Option<u8>,
    pub files: Vec<ThumbFile>,
}

#[derive(Serialize, Clone)]
pub struct Thumb {
    pub name: String,
    pub source: Option<&'static str>,
    pub picture: Option<String>,
    pub image: Option<String>,
    pub error: Option<String>,
    pub ms: u64,
}

#[derive(Serialize)]
pub struct ThumbsBody {
    pub ok: bool,
    pub device: String,
    pub elapsed_ms: u64,
    pub thumbs: Vec<Thumb>,
}

type ThumbCache = Mutex<HashMap<String, Thumb>>;

fn cache() -> &'static ThumbCache {
    static CACHE: OnceLock<ThumbCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached(key: &str) -> Option<Thumb> {
    cache().lock().ok().and_then(|map| map.get(key).cloned())
}

fn remember(key: String, thumb: &Thumb) {
    if let Ok(mut map) = cache().lock() {
        if map.len() >= CACHE_LIMIT {
            map.clear();
        }
        map.insert(key, thumb.clone());
    }
}

fn demuxer(codec: Codec) -> &'static str {
    match codec {
        Codec::H264 => "h264",
        Codec::H265 => "hevc",
    }
}

pub async fn decode_keyframe(codec: Codec, annex_b: &[u8]) -> Result<Vec<u8>, String> {
    let binary = std::env::var("PROVIEW_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string());
    let mut command = Command::new(binary);
    proview_core::process::hide_console(command.as_std_mut());
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            demuxer(codec),
            "-i",
            "pipe:0",
        ])
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=160:-2",
            "-f",
            "mjpeg",
            "-q:v",
            "5",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| format!("ffmpeg did not start: {error}"))?;
    let mut stdin = child.stdin.take().ok_or("ffmpeg has no stdin")?;
    let mut stdout = child.stdout.take().ok_or("ffmpeg has no stdout")?;
    let input = annex_b.to_vec();
    let feed = tokio::spawn(async move {
        let _ = stdin.write_all(&input).await;
        let _ = stdin.shutdown().await;
    });
    let mut jpeg = Vec::new();
    let read = tokio::time::timeout(DECODE_WAIT, stdout.read_to_end(&mut jpeg)).await;
    feed.abort();
    let _ = child.start_kill();
    let _ = child.wait().await;
    match read {
        Ok(Ok(_)) if is_jpeg(&jpeg) => Ok(jpeg),
        Ok(Ok(_)) => Err("ffmpeg produced no picture from the keyframe".into()),
        Ok(Err(error)) => Err(format!("ffmpeg output failed: {error}")),
        Err(_) => Err(format!(
            "ffmpeg did not decode the keyframe inside {:.0} s",
            DECODE_WAIT.as_secs_f64()
        )),
    }
}

pub async fn first_keyframe(
    session: &mut Session,
    file: &PlaybackFile,
    channel: u8,
    wait: Duration,
) -> Result<(Codec, Vec<u8>), String> {
    session
        .claim(file, channel)
        .await
        .map_err(|error| format!("claim: {error}"))?;
    session
        .start(file, channel)
        .await
        .map_err(|error| format!("start: {error}"))?;
    let deadline = tokio::time::Instant::now() + wait;
    let mut reader = MediaReader::new();
    let found = loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break Err(format!("no keyframe inside {:.0} s", wait.as_secs_f64()));
        }
        match session.frame(left).await {
            Ok(Some(frame)) if frame.id == PLAYBACK_DATA => {
                reader.push(&frame.payload);
                let mut key = None;
                while let Some(record) = reader.next_record() {
                    if record.kind == RecordKind::Key {
                        key = Some(record);
                        break;
                    }
                }
                if let Some(record) = key {
                    let nals = access_unit(&record);
                    match codec_of(&nals) {
                        Some(codec) => {
                            let mut out = Vec::new();
                            for nal in &nals {
                                out.extend_from_slice(&[0, 0, 0, 1]);
                                out.extend_from_slice(nal);
                            }
                            break Ok((codec, out));
                        }
                        None => break Err("the keyframe carried no parameter sets".into()),
                    }
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) => break Err(format!("no keyframe inside {:.0} s", wait.as_secs_f64())),
            Err(error) => break Err(error.to_string()),
        }
    };
    let _ = session.stop(file, channel).await;
    found
}

fn clip_of(file: &ThumbFile) -> PlaybackFile {
    PlaybackFile {
        name: file.name.clone(),
        begin: file.begin.clone(),
        end: file.end.clone(),
        bytes: 0,
        disk: None,
    }
}

struct Draft {
    thumb: Thumb,
    reasons: Vec<String>,
    spent: Duration,
}

impl Draft {
    fn new(file: &ThumbFile) -> Self {
        Self {
            thumb: Thumb {
                name: file.name.clone(),
                source: None,
                picture: None,
                image: None,
                error: None,
                ms: 0,
            },
            reasons: Vec::new(),
            spent: Duration::ZERO,
        }
    }

    fn finish(mut self) -> Thumb {
        if self.thumb.image.is_none() {
            self.thumb.error = Some(if self.reasons.is_empty() {
                "no picture for this file".into()
            } else {
                self.reasons.join("; ")
            });
        }
        self.thumb.ms = self.spent.as_millis() as u64;
        self.thumb
    }
}

async fn picture_thumb(
    session: &mut Session,
    file: &ThumbFile,
    pictures: &[PlaybackFile],
    draft: &mut Draft,
) {
    let picture = if file.name.ends_with(".jpg") {
        Some(file.name.clone())
    } else {
        picture_inside(&clip_of(file), pictures).map(|picture| picture.name.clone())
    };
    let Some(name) = picture else {
        return;
    };
    let started = Instant::now();
    let answer = session.compressed_picture(&name, PICTURE_WAIT).await;
    draft.spent += started.elapsed();
    match answer {
        Ok(jpeg) => {
            draft.thumb.source = Some("picture");
            draft.thumb.picture = Some(name);
            draft.thumb.image = Some(data_url(&jpeg));
        }
        Err(error) => draft.reasons.push(format!("picture {name}: {error}")),
    }
}

async fn keyframe_thumb(session: &mut Session, file: &ThumbFile, channel: u8, draft: &mut Draft) {
    match first_keyframe(session, &clip_of(file), channel, KEYFRAME_WAIT).await {
        Ok((codec, annex_b)) => match decode_keyframe(codec, &annex_b).await {
            Ok(jpeg) => {
                draft.thumb.source = Some("keyframe");
                draft.thumb.image = Some(data_url(&jpeg));
            }
            Err(error) => draft.reasons.push(format!("keyframe decode: {error}")),
        },
        Err(error) => draft.reasons.push(format!("keyframe: {error}")),
    }
}

pub fn keyframe_fits(left: Duration) -> bool {
    left >= KEYFRAME_RESERVE
}

pub fn batch_left(started: Instant, limit: Duration) -> Duration {
    limit.saturating_sub(started.elapsed())
}

async fn close(mut session: Session) {
    let _ = tokio::time::timeout(Duration::from_secs(2), session.logout()).await;
}

#[post("/playback/thumbnails", data = "<ask>")]
pub async fn thumbnails(
    ask: Json<ThumbAsk>,
    cloud: &State<Arc<CloudProvider>>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<ThumbsBody>, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    let ask = ask.into_inner();
    valid_device(&ask.device)?;
    if ask.files.is_empty() || ask.files.len() > MAX_THUMBS {
        return Err(fail(
            Status::BadRequest,
            format!("ask for 1 to {MAX_THUMBS} files at a time"),
        ));
    }
    let channel = ask.channel.unwrap_or(0);
    let started = Instant::now();
    let key = |name: &str| format!("{}|{channel}|{name}", ask.device);
    let mut thumbs: Vec<Option<Thumb>> = ask
        .files
        .iter()
        .map(|file| cached(&key(&file.name)))
        .collect();
    if thumbs.iter().any(Option::is_none) {
        let wait = window();
        let budget = BATCH_DEADLINE;
        let device = ask.device.clone();
        let files = &ask.files;
        let pending: Vec<usize> = (0..files.len())
            .filter(|&index| thumbs[index].is_none())
            .collect();
        let work = async {
            let mut session =
                open_tunnel(cloud.inner(), &vendor, &device, wait.min(budget)).await?;
            let clips: Vec<&ThumbFile> = pending
                .iter()
                .map(|&index| &files[index])
                .filter(|file| !file.name.ends_with(".jpg"))
                .collect();
            let pictures = match (
                clips.iter().map(|file| file.begin.as_str()).min(),
                clips.iter().map(|file| file.end.as_str()).max(),
            ) {
                (Some(begin), Some(end)) => within(
                    "picture-query",
                    wait,
                    session.files_by(channel, begin, end, "jpg", RecordFilter::All),
                )
                .await
                .unwrap_or_default(),
                _ => Vec::new(),
            };
            let mut drafts = Vec::new();
            for &index in &pending {
                let mut draft = Draft::new(&files[index]);
                picture_thumb(&mut session, &files[index], &pictures, &mut draft).await;
                drafts.push((index, draft));
            }
            let mut spare = Some(session);
            for (index, draft) in drafts.iter_mut() {
                let file = &files[*index];
                if draft.thumb.image.is_some() || file.name.ends_with(".jpg") {
                    continue;
                }
                let left = batch_left(started, budget);
                if !keyframe_fits(left) {
                    draft.reasons.push(format!(
                        "keyframe: skipped, a thumbnail batch spends at most {} s",
                        budget.as_secs()
                    ));
                    continue;
                }
                let begun = Instant::now();
                let opened = match spare.take() {
                    Some(session) => Ok(session),
                    None => open_tunnel(cloud.inner(), &vendor, &device, wait.min(left)).await,
                };
                match opened {
                    Ok(mut session) => {
                        let left = batch_left(started, budget);
                        if tokio::time::timeout(
                            left,
                            keyframe_thumb(&mut session, file, channel, draft),
                        )
                        .await
                        .is_err()
                        {
                            draft.reasons.push(format!(
                                "keyframe: the batch deadline of {} s ran out",
                                budget.as_secs()
                            ));
                        }
                        close(session).await;
                    }
                    Err(failure) => draft
                        .reasons
                        .push(format!("keyframe: {}", failure.1.message)),
                }
                draft.spent += begun.elapsed();
            }
            if let Some(session) = spare {
                close(session).await;
            }
            Ok::<_, Failure>(drafts)
        };
        let drafts = match tokio::time::timeout(budget, work).await {
            Ok(result) => result?,
            Err(_) => return Err(too_slow("thumbnails", budget)),
        };
        for (index, draft) in drafts {
            let thumb = draft.finish();
            if thumb.image.is_some() {
                remember(key(&thumb.name), &thumb);
            }
            thumbs[index] = Some(thumb);
        }
    }
    Ok(Json(ThumbsBody {
        ok: true,
        device: ask.device.clone(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        thumbs: thumbs.into_iter().flatten().collect(),
    }))
}

#[derive(Default)]
pub struct Pace {
    position: AtomicUsize,
    heard: AtomicBool,
    notify: Notify,
}

#[derive(Deserialize)]
struct Position {
    #[serde(rename = "type")]
    kind: String,
    frame: usize,
}

impl Pace {
    pub fn heard(&self, text: &str) -> bool {
        match serde_json::from_str::<Position>(text) {
            Ok(position) if position.kind == "position" => {
                self.position.store(position.frame, Ordering::SeqCst);
                self.heard.store(true, Ordering::SeqCst);
                self.notify.notify_waiters();
                true
            }
            _ => false,
        }
    }

    pub fn release(&self) {
        self.heard.store(false, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn ahead(&self, total: usize, lead: usize) -> bool {
        self.heard.load(Ordering::SeqCst)
            && total > self.position.load(Ordering::SeqCst).saturating_add(lead)
    }

    pub async fn admit(&self, total: usize, lead: usize) {
        while self.ahead(total, lead) {
            let woken = self.notify.notified();
            if !self.ahead(total, lead) {
                break;
            }
            let _ = tokio::time::timeout(Duration::from_secs(1), woken).await;
        }
    }
}

pub fn lead_frames(fps: Option<f64>) -> usize {
    (PACE_LEAD_SECONDS * fps.unwrap_or(25.0)).ceil() as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use proview_core::config::{Config, ProviderKind};
    use rocket::local::blocking::Client;

    fn client(signed: bool) -> Client {
        let config = Config {
            provider: ProviderKind::Fixture,
            cloud_base: "https://pub-token.example.invalid".to_string(),
            share_base: proview_core::cloud::attributes::SHARE_BASE.to_string(),
            device_user: "admin".to_string(),
            device_pass: "device-password".to_string(),
            fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
            device_host: None,
            device_onvif: None,
            device_rtsp: None,
            session_ttl_secs: 60,
        };
        let figment = rocket::Config::figment()
            .merge(("secret_key", "uVo3haa/znSxx4MPMV95m4oe9EYQe99QbYgEuBtVqqU="))
            .merge(("port", 0));
        let client = Client::tracked(crate::app::build_with(figment, config)).expect("rocket");
        if signed {
            let status = client
                .post("/api/login")
                .json(&serde_json::json!({
                    "email": "fixture@example.invalid",
                    "password": "fixture-password"
                }))
                .dispatch()
                .status();
            assert_eq!(status, Status::Ok);
        }
        client
    }

    fn files(count: usize) -> serde_json::Value {
        let files: Vec<serde_json::Value> = (0..count)
            .map(|index| {
                serde_json::json!({
                    "name": format!("/idea0/2026-10-30/001/00.0{index}.00-00.0{index}.30[R][@1][0].h264"),
                    "begin": "2026-10-30 00:00:00",
                    "end": "2026-10-30 00:00:30",
                })
            })
            .collect();
        serde_json::json!({ "device": "serial-one", "channel": 0, "files": files })
    }

    #[test]
    fn the_calendar_and_thumbnails_refuse_a_request_without_a_session() {
        let rocket = client(false);
        let calendar = rocket
            .get("/api/playback/calendar?device=serial-one&year=2026&month=10")
            .dispatch();
        assert_eq!(calendar.status(), Status::Unauthorized);
        let thumbs = rocket
            .post("/api/playback/thumbnails")
            .json(&files(1))
            .dispatch();
        assert_eq!(thumbs.status(), Status::Unauthorized);
    }

    #[test]
    fn a_month_or_device_out_of_range_is_refused_before_the_tunnel() {
        let rocket = client(true);
        for path in [
            "/api/playback/calendar?device=serial-one&year=2026&month=13",
            "/api/playback/calendar?device=serial-one&year=2026&month=0",
            "/api/playback/calendar?device=serial-one&year=1999&month=5",
            "/api/playback/calendar?device=../x&year=2026&month=5",
        ] {
            assert_eq!(
                rocket.get(path).dispatch().status(),
                Status::BadRequest,
                "{path}"
            );
        }
    }

    #[test]
    fn thumbnails_are_asked_one_to_eight_at_a_time() {
        let rocket = client(true);
        let none = rocket
            .post("/api/playback/thumbnails")
            .json(&files(0))
            .dispatch();
        assert_eq!(none.status(), Status::BadRequest);
        let many = rocket
            .post("/api/playback/thumbnails")
            .json(&files(MAX_THUMBS + 1))
            .dispatch();
        assert_eq!(many.status(), Status::BadRequest);
        let text = many.into_string().unwrap_or_default();
        assert!(text.contains("1 to 6 files"), "{text}");
    }

    #[test]
    fn an_unknown_record_filter_is_a_bad_request() {
        let rocket = client(true);
        let response = rocket
            .get("/api/playback/files?device=serial-one&channel=0&begin=2026-10-30%2000:00:00&end=2026-10-30%2023:59:59&event=bogus")
            .dispatch();
        assert_eq!(response.status(), Status::BadRequest);
        assert!(response
            .into_string()
            .unwrap_or_default()
            .contains("event must be all, regular or alarm"));
    }

    #[tokio::test]
    async fn a_silent_device_answers_504_naming_the_stage_inside_the_window() {
        let wait = Duration::from_millis(200);
        let started = Instant::now();
        let failure = within(
            "calendar",
            wait,
            std::future::pending::<Result<Vec<u32>, PlaybackError>>(),
        )
        .await
        .unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(900));
        assert_eq!(failure.0, Status::GatewayTimeout);
        assert!(
            failure.1.message.contains("stage=calendar"),
            "{}",
            failure.1.message
        );
        assert!(failure.1.message.contains("0.2 s"), "{}", failure.1.message);
    }

    #[tokio::test]
    async fn a_refusal_is_502_and_a_device_timeout_is_504() {
        let refused = within::<()>("file-query", Duration::from_secs(1), async {
            Err(PlaybackError::Rejected {
                ret: 103,
                reason: "request not permitted",
            })
        })
        .await
        .unwrap_err();
        assert_eq!(refused.0, Status::BadGateway);
        assert!(refused.1.message.contains("stage=file-query"));
        assert!(refused.1.message.contains("103"));
        let silent = within::<()>("thumbnails", Duration::from_secs(1), async {
            Err(PlaybackError::NoReply(Duration::from_secs(6)))
        })
        .await
        .unwrap_err();
        assert_eq!(silent.0, Status::GatewayTimeout);
    }

    #[tokio::test]
    async fn the_pump_waits_for_the_viewer_only_once_it_reports_a_position() {
        let pace = Arc::new(Pace::default());
        tokio::time::timeout(Duration::from_millis(100), pace.admit(10_000, 240))
            .await
            .expect("no position yet means no pacing");
        assert!(!pace.heard(r#"{"type":"speed","frame":3}"#));
        assert!(pace.heard(r#"{"type":"position","frame":0}"#));
        assert!(pace.ahead(241, 240));
        assert!(!pace.ahead(240, 240));
        let waiter = {
            let pace = pace.clone();
            tokio::spawn(async move { pace.admit(300, 240).await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!waiter.is_finished());
        assert!(pace.heard(r#"{"type":"position","frame":60}"#));
        tokio::time::timeout(Duration::from_millis(500), waiter)
            .await
            .expect("a viewer that caught up lets the pump go")
            .unwrap();
        let stuck = {
            let pace = pace.clone();
            tokio::spawn(async move { pace.admit(10_000, 240).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        pace.release();
        tokio::time::timeout(Duration::from_millis(500), stuck)
            .await
            .expect("a closed socket releases the pump")
            .unwrap();
    }

    #[test]
    fn a_batch_stops_starting_keyframes_before_its_deadline() {
        assert_eq!(BATCH_DEADLINE, Duration::from_secs(60));
        let fresh = Instant::now();
        assert!(keyframe_fits(batch_left(fresh, BATCH_DEADLINE)));
        let late = Instant::now() - Duration::from_secs(50);
        assert!(!keyframe_fits(batch_left(late, BATCH_DEADLINE)));
        let over = Instant::now() - Duration::from_secs(90);
        assert_eq!(batch_left(over, BATCH_DEADLINE), Duration::ZERO);
        assert!(KEYFRAME_RESERVE >= KEYFRAME_WAIT);
    }

    #[test]
    fn the_lead_is_twenty_seconds_of_frames() {
        assert_eq!(lead_frames(Some(12.0)), 240);
        assert_eq!(lead_frames(None), 500);
    }
}
