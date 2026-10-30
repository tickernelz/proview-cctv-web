use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use proview_core::dvrip::playback::{
    access_unit, codec_of, is_video, shift, snap_rate, FrameRate, MediaReader, PlaybackFile,
    RecordKind, Session, DOWNLOAD_DATA, PLAYBACK_DATA,
};
use proview_core::dvrip::playback_plus::{PlayMode, PlaybackSide, RecordFilter};
use proview_core::media::hevc;
use proview_core::media::mux::ClipMux;
use proview_core::media::Codec;
use proview_core::provider::cloud::CloudProvider;
use proview_core::provider::{ProviderError, VendorSession};
use proview_core::session::Store;
use proview_core::source::Piece;
use rocket::http::{ContentType, CookieJar, Status};
use rocket::request::Request;
use rocket::response::{self, Responder, Response};
use rocket::serde::json::Json;
use rocket::State;
use rocket_ws as ws;
use serde::{Deserialize, Serialize};

use crate::routes::playback_mux::demuxer;
use crate::routes::playback_plus::{self, lead_frames, Pace};
use crate::routes::{fail, restore, session_id, status_for, Failure};

const PRIME_READ: Duration = Duration::from_secs(3);
const PRIME_WAIT: Duration = Duration::from_secs(8);
const DEFAULT_WINDOW: u32 = 60;
const MIN_WINDOW: u32 = 5;
const MAX_WINDOW: u32 = 600;
const MAX_TIME_WINDOW: u32 = 3600;
const DEFAULT_FPS: f64 = 12.0;
const KEY_GRACE: usize = 240;
const DOWNLOAD_GAP: Duration = Duration::from_secs(20);
const DOWNLOAD_LIMIT: u64 = 512 * 1024 * 1024;

pub fn routes() -> Vec<rocket::route::Route> {
    routes![files]
}

#[derive(Serialize)]
pub struct FilesBody {
    pub ok: bool,
    pub source: &'static str,
    pub device: String,
    pub channel: u8,
    pub event: &'static str,
    pub begin: String,
    pub end: String,
    pub count: usize,
    pub bytes: u64,
    pub truncated: bool,
    pub cap: usize,
    pub elapsed_ms: u64,
    pub files: Vec<PlaybackFile>,
}

#[get("/playback/files?<device>&<channel>&<begin>&<end>&<kind>&<event>")]
#[allow(clippy::too_many_arguments)]
pub async fn files(
    device: String,
    channel: Option<u8>,
    begin: Option<String>,
    end: Option<String>,
    kind: Option<String>,
    event: Option<String>,
    cloud: &State<Arc<CloudProvider>>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<FilesBody>, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    if device.contains('/') || device.contains("..") {
        return Err(fail(Status::BadRequest, "invalid device id"));
    }
    let begin = begin
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            fail(
                Status::BadRequest,
                "begin is required as 'YYYY-MM-DD HH:MM:SS' in device time",
            )
        })?;
    let end = end
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            fail(
                Status::BadRequest,
                "end is required as 'YYYY-MM-DD HH:MM:SS' in device time",
            )
        })?;
    let channel = channel.unwrap_or(0);
    let kind = kind.unwrap_or_else(|| "h264".to_string());
    let filter = RecordFilter::parse(event.as_deref().unwrap_or(""))
        .ok_or_else(|| fail(Status::BadRequest, "event must be all, regular or alarm"))?;
    let started = Instant::now();
    let wait = playback_plus::window();
    let mut session = playback_plus::open_tunnel(cloud.inner(), &vendor, &device, wait).await?;
    let found = playback_plus::within(
        "file-query",
        wait,
        session.files_by(channel, &begin, &end, &kind, filter),
    )
    .await?;
    let _ = session.logout().await;
    let bytes = found.iter().map(|file| file.bytes).sum();
    Ok(Json(FilesBody {
        ok: true,
        source: "cloud",
        device,
        channel,
        event: filter.wire(),
        begin,
        end,
        count: found.len(),
        bytes,
        truncated: found.len() >= proview_core::dvrip::playback::MAX_FILES,
        cap: proview_core::dvrip::playback::MAX_FILES,
        elapsed_ms: started.elapsed().as_millis() as u64,
        files: found,
    }))
}

pub struct Download {
    bytes: Vec<u8>,
    filename: String,
    kind: ContentType,
}

impl<'r> Responder<'r, 'static> for Download {
    fn respond_to(self, _request: &'r Request<'_>) -> response::Result<'static> {
        Response::build()
            .header(self.kind)
            .raw_header(
                "Content-Disposition",
                format!("attachment; filename=\"{}\"", self.filename),
            )
            .raw_header("Cache-Control", "no-store")
            .sized_body(self.bytes.len(), std::io::Cursor::new(self.bytes))
            .ok()
    }
}

fn short_fall(wire: u64, listed: u64) -> Option<String> {
    if wire >= listed {
        None
    } else {
        Some(format!("the device sent {wire} of {listed} bytes"))
    }
}

async fn collect(session: &mut Session, listed: u64, mux: &mut ClipMux) -> Result<(), String> {
    let mut reader = MediaReader::new();
    let mut wire = 0u64;
    loop {
        while let Some(record) = reader.next_record() {
            mux.push(record);
            if mux.held() as u64 > DOWNLOAD_LIMIT {
                return Err(format!(
                    "the file is larger than the {DOWNLOAD_LIMIT} bytes this server holds"
                ));
            }
        }
        if short_fall(wire, listed).is_none() {
            break;
        }
        match session.frame(DOWNLOAD_GAP).await {
            Ok(Some(frame)) => {
                if frame.id != PLAYBACK_DATA && frame.id != DOWNLOAD_DATA {
                    continue;
                }
                wire += frame.payload.len() as u64;
                reader.push(&frame.payload);
            }
            Ok(None) => {
                return Err(short_fall(wire, listed).unwrap_or_else(|| "the device stopped".into()))
            }
            Err(error) => {
                return Err(format!(
                    "the tunnel broke after {wire} of {listed} bytes: {error}"
                ))
            }
        }
    }
    Ok(())
}

#[get("/playback/download?<device>&<channel>&<name>&<begin>&<end>&<bytes>")]
#[allow(clippy::too_many_arguments)]
pub async fn download(
    device: String,
    channel: Option<u8>,
    name: String,
    begin: String,
    end: String,
    bytes: Option<u64>,
    cloud: &State<Arc<CloudProvider>>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Download, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    if device.contains('/') || device.contains("..") {
        return Err(fail(Status::BadRequest, "invalid device id"));
    }
    if !name.starts_with('/') || name.contains("..") {
        return Err(fail(Status::BadRequest, "name must be a device file path"));
    }
    let listed = bytes
        .filter(|value| *value > 0 && *value <= DOWNLOAD_LIMIT)
        .ok_or_else(|| {
            fail(
                Status::BadRequest,
                format!("bytes is required: FileLength x 1024, at most {DOWNLOAD_LIMIT}"),
            )
        })?;
    let channel = channel.unwrap_or(0);
    let file = PlaybackFile {
        name: name.clone(),
        begin: begin.clone(),
        end: end.clone(),
        bytes: 0,
        disk: None,
    };
    let mut session = open_tunnel(cloud.inner(), &vendor, &device)
        .await
        .map_err(|error| fail(status_for(&error), error.to_string()))?;
    session
        .claim(&file, channel)
        .await
        .map_err(|error| fail(Status::BadGateway, format!("playback stage=claim: {error}")))?;
    session
        .download_start(&file, channel)
        .await
        .map_err(|error| fail(Status::BadGateway, error.to_string()))?;
    let mut mux = ClipMux::new(listed as usize);
    let collected = collect(&mut session, listed, &mut mux).await;
    let _ = session.download_stop(&file, channel).await;
    let _ = session.logout().await;
    collected.map_err(|message| fail(Status::BadGateway, message))?;
    let clip = tokio::task::spawn_blocking(move || mux.finish())
        .await
        .map_err(|error| {
            fail(
                Status::InternalServerError,
                format!("playback stage=mux: {error}"),
            )
        })?
        .map_err(|message| fail(Status::BadGateway, format!("playback stage=mux: {message}")))?;
    tracing::info!(
        codec = demuxer(clip.codec),
        muxed = clip.muxed,
        frames = clip.frames,
        fps = clip.fps,
        fps_source = clip.fps_source,
        seconds = clip.seconds,
        "playback download ready"
    );
    let (extension, kind) = if clip.muxed {
        ("mp4", ContentType::MP4)
    } else {
        (demuxer(clip.codec), ContentType::Binary)
    };
    Ok(Download {
        bytes: clip.bytes,
        filename: download_name(&name, &begin, extension),
        kind,
    })
}

fn clean_name(text: &str) -> String {
    text.chars()
        .map(|letter| {
            if letter.is_ascii_alphanumeric() || matches!(letter, '.' | '_' | '-' | '@' | '[' | ']')
            {
                letter
            } else {
                '_'
            }
        })
        .collect()
}

fn download_name(name: &str, begin: &str, extension: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    let stem = match base.rfind('.') {
        Some(dot) if dot > 0 => &base[..dot],
        _ => base,
    };
    let stem = clean_name(stem);
    let day = begin.split(' ').next().unwrap_or("");
    let dated = day.len() == 10
        && day.chars().enumerate().all(|(index, letter)| {
            if index == 4 || index == 7 {
                letter == '-'
            } else {
                letter.is_ascii_digit()
            }
        });
    match (stem.is_empty(), dated) {
        (true, true) => format!("{day}_clip.{extension}"),
        (true, false) => format!("clip.{extension}"),
        (false, true) => format!("{day}_{stem}.{extension}"),
        (false, false) => format!("{stem}.{extension}"),
    }
}

async fn open_tunnel(
    cloud: &CloudProvider,
    vendor: &VendorSession,
    serial: &str,
) -> Result<Session, ProviderError> {
    let (session, note) = cloud.open_playback(vendor, serial).await?;
    tracing::debug!(device = %serial, %note, "playback device login");
    Ok(session)
}

#[derive(Deserialize)]
struct WatchQuery {
    device: String,
    channel: Option<u8>,
    name: String,
    begin: String,
    end: String,
    start: Option<u32>,
    window: Option<u32>,
    codec: Option<String>,
    #[serde(skip)]
    mode: PlayMode,
}

type RateCache = std::sync::Mutex<std::collections::HashMap<String, f64>>;

fn known_rates() -> &'static RateCache {
    static RATES: std::sync::OnceLock<RateCache> = std::sync::OnceLock::new();
    RATES.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn known_rate(device: &str) -> Option<f64> {
    known_rates()
        .lock()
        .ok()
        .and_then(|rates| rates.get(device).copied())
}

fn remember_rate(device: &str, fps: f64) {
    if let Ok(mut rates) = known_rates().lock() {
        rates.insert(device.to_string(), fps);
    }
}

fn annex_b(record: &proview_core::dvrip::playback::Record, out: &mut Vec<u8>) -> bool {
    if !is_video(record) {
        return false;
    }
    let nals = access_unit(record);
    if nals.is_empty() {
        return false;
    }
    for nal in &nals {
        hevc::append_annexb(nal, out);
    }
    true
}

fn text_frame(payload: serde_json::Value) -> ws::Message {
    ws::Message::Text(payload.to_string())
}

#[get("/ws/playback?<device>&<channel>&<name>&<begin>&<end>&<start>&<window>&<codec>&<mode>")]
#[allow(clippy::too_many_arguments)]
pub fn watch(
    ws: ws::WebSocket,
    device: String,
    channel: Option<u8>,
    name: String,
    begin: String,
    end: String,
    start: Option<u32>,
    window: Option<u32>,
    codec: Option<String>,
    mode: Option<String>,
    cloud: &State<Arc<CloudProvider>>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<ws::Channel<'static>, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    if device.contains('/') || device.contains("..") {
        return Err(fail(Status::BadRequest, "invalid device id"));
    }
    let mode = PlayMode::parse(mode.as_deref())
        .ok_or_else(|| fail(Status::BadRequest, "mode must be file or time"))?;
    let params = WatchQuery {
        device,
        channel,
        name,
        begin,
        end,
        start,
        window,
        codec,
        mode,
    };
    let cloud = cloud.inner().clone();
    Ok(ws.channel(move |mut stream| {
        Box::pin(async move {
            let channel = params.channel.unwrap_or(0);
            let from = params.start.unwrap_or(0);
            let play = params.mode;
            let ceiling = match play {
                PlayMode::ByName => MAX_WINDOW,
                PlayMode::ByTime => MAX_TIME_WINDOW,
            };
            let window = params
                .window
                .unwrap_or(DEFAULT_WINDOW)
                .clamp(MIN_WINDOW, ceiling);
            let side = Arc::new(std::sync::Mutex::new(PlaybackSide::default()));
            let pace = Arc::new(Pace::default());
            let file = PlaybackFile {
                name: params.name.clone(),
                begin: shift(&params.begin, from).unwrap_or_else(|| params.begin.clone()),
                end: params.end.clone(),
                bytes: 0,
                disk: None,
            };
            let _ = stream
                .send(text_frame(serde_json::json!({
                    "type": "status",
                    "message": "opening the cloud tunnel",
                })))
                .await;
            let opened = Instant::now();
            let mut session = match open_tunnel(&cloud, &vendor, &params.device).await {
                Ok(session) => session,
                Err(error) => {
                    let _ = stream
                        .send(text_frame(serde_json::json!({
                            "type": "error",
                            "message": error.to_string(),
                        })))
                        .await;
                    return Ok(());
                }
            };
            let _ = stream
                .send(text_frame(serde_json::json!({
                    "type": "status",
                    "message": "asking the device for the clip",
                    "openMs": opened.elapsed().as_millis() as u64,
                })))
                .await;
            if let Err(error) = session.claim_in(play, &file, channel).await {
                let _ = stream
                    .send(text_frame(serde_json::json!({
                        "type": "error",
                        "message": format!("playback stage=claim: {error}"),
                        "stage": "claim",
                    })))
                    .await;
                return Ok(());
            }
            let queued = Instant::now();
            if let Err(error) = session.start_in(play, &file, channel).await {
                let _ = stream
                    .send(text_frame(serde_json::json!({
                        "type": "error",
                        "message": error.to_string(),
                    })))
                    .await;
                return Ok(());
            }

            let mut reader = MediaReader::new();
            let mut rate = FrameRate::default();
            let mut pending: Vec<Vec<u8>> = Vec::new();
            let mut key_frames = 0usize;
            let mut video = 0usize;
            let mut codec = None;
            let mut first_key = Instant::now();
            loop {
                if rate.settled() || video >= KEY_GRACE || first_key.elapsed() > PRIME_WAIT {
                    break;
                }
                let record = match session.frame(PRIME_READ).await {
                    Ok(Some(record)) => record,
                    Ok(None) => {
                        break;
                    }
                    Err(error) => {
                        if key_frames == 0 {
                            let _ = stream
                                .send(text_frame(serde_json::json!({
                                    "type": "error",
                                    "message": error.to_string(),
                                })))
                                .await;
                            return Ok(());
                        }
                        break;
                    }
                };
                if record.id != PLAYBACK_DATA {
                    continue;
                }
                reader.push(&record.payload);
                while let Some(frame) = reader.next_record() {
                    if let Ok(mut side) = side.lock() {
                        side.observe(&frame, video);
                    }
                    if frame.kind == RecordKind::Key {
                        key_frames += 1;
                        if key_frames == 1 {
                            first_key = Instant::now();
                            codec = codec_of(&access_unit(&frame));
                        }
                    }
                    let mut bytes = Vec::new();
                    if annex_b(&frame, &mut bytes) {
                        video += 1;
                        rate.observe(&frame);
                        pending.push(bytes);
                    }
                }
            }

            let codec = match codec {
                Some(codec) => codec,
                None => {
                    let message = if key_frames == 0 {
                        "the device streamed no keyframe inside the priming window"
                    } else {
                        "the keyframe carried no parameter sets, so the codec is unknown"
                    };
                    let _ = stream
                        .send(text_frame(serde_json::json!({
                            "type": "error",
                            "message": message,
                        })))
                        .await;
                    return Ok(());
                }
            };
            let remembered = known_rate(&params.device);
            let (fps, fps_source) = if rate.settled() {
                remember_rate(&params.device, rate.per_second);
                (Some(rate.per_second), "settled")
            } else if let Some(known) = remembered {
                (Some(known), "remembered")
            } else if rate.keys() >= 3 {
                (rate.estimate().map(snap_rate), "estimate")
            } else {
                (Some(DEFAULT_FPS), "default")
            };
            let config = if crate::routes::playback_mux::wants_native(codec, params.codec.as_deref()) {
                crate::routes::playback_mux::config_of(codec, &pending)
            } else {
                None
            };
            let pipeline = match config.as_ref() {
                Some(config) => Ok(crate::routes::playback_mux::native(
                    codec,
                    config,
                    fps.unwrap_or(DEFAULT_FPS),
                )),
                None => crate::routes::playback_mux::transcode(codec, fps),
            };
            let crate::routes::playback_mux::Pipeline {
                mut sink,
                mut output,
                mut child,
                stderr,
            } = match pipeline {
                Ok(pipeline) => pipeline,
                Err(message) => {
                    let _ = stream
                        .send(text_frame(serde_json::json!({
                            "type": "error",
                            "message": message,
                        })))
                        .await;
                    return Ok(());
                }
            };
            let native = config.is_some();
            let (mime, transport) = match config.as_ref() {
                Some(config) => (
                    config.mime(),
                    if codec == Codec::H265 { "hevc" } else { "avc" },
                ),
                None => (crate::routes::playback_mux::AVC_MIME.to_string(), "avc"),
            };
            let _ = stream
                .send(text_frame(serde_json::json!({
                    "type": "status",
                    "message": if native { "muxing the clip without a transcode" } else { "transcoding the clip" },
                    "codec": demuxer(codec),
                    "pipeline": if native { "native" } else { "ffmpeg" },
                    "transport": transport,
                    "mime": mime,
                    "codecString": config.as_ref().map(|config| config.codec_string.clone()),
                    "width": config.as_ref().map(|config| config.width),
                    "height": config.as_ref().map(|config| config.height),
                    "fps": fps.map(|value| (value * 1000.0).round() / 1000.0),
                    "fpsSettled": rate.settled(),
                    "fpsSource": fps_source,
                    "fpsMeasured": rate.estimate().map(|value| (value * 1000.0).round() / 1000.0),
                    "keySpan": rate.span_seconds(),
                    "primedFrames": video,
                    "primedKeys": key_frames,
                    "primeMs": queued.elapsed().as_millis() as u64,
                    "windowSec": window,
                    "mode": play.wire(),
                })))
                .await;

            let primed = pending;
            let primed_count = primed.len();
            let stop_file = file.clone();
            let window_frames = (f64::from(window) * fps.unwrap_or(25.0)).ceil() as usize;
            let lead = lead_frames(fps);
            let pump_side = side.clone();
            let pump_pace = pace.clone();
            let pump = tokio::spawn(async move {
                for frame in primed {
                    if !sink.unit(&frame).await {
                        return (video, false);
                    }
                }
                let mut written = 0usize;
                let mut capped = false;
                loop {
                    if written >= window_frames {
                        capped = true;
                        break;
                    }
                    pump_pace.admit(primed_count + written, lead).await;
                    let record = match session.frame(Duration::from_secs(10)).await {
                        Ok(Some(record)) => record,
                        _ => break,
                    };
                    if record.id != PLAYBACK_DATA {
                        continue;
                    }
                    reader.push(&record.payload);
                    let mut keep = true;
                    while let Some(frame) = reader.next_record() {
                        if let Ok(mut side) = pump_side.lock() {
                            side.observe(&frame, primed_count + written);
                        }
                        let mut bytes = Vec::new();
                        if annex_b(&frame, &mut bytes) {
                            written += 1;
                            if !sink.unit(&bytes).await {
                                keep = false;
                                break;
                            }
                        }
                    }
                    if !keep {
                        break;
                    }
                }
                sink.finish().await;
                let _ = session.stop_in(play, &stop_file, channel).await;
                if let Ok(mut side) = pump_side.lock() {
                    side.flush();
                }
                (written, capped)
            });

            let mut init_sent = false;
            let mut sent = 0u64;
            let mut first_frame: Option<u64> = None;
            let mut closed_by_client = false;
            'relay: loop {
                let piece = tokio::select! {
                    biased;
                    incoming = stream.next() => {
                        match incoming {
                            None | Some(Err(_)) | Some(Ok(ws::Message::Close(_))) => {
                                closed_by_client = true;
                                break 'relay;
                            }
                            Some(Ok(ws::Message::Text(text))) => {
                                pace.heard(&text);
                                continue;
                            }
                            Some(Ok(_)) => continue,
                        }
                    }
                    piece = output.next() => match piece {
                        Some(piece) => piece,
                        None => break,
                    },
                };
                let payload = match piece {
                    Piece::Init(fragment) => {
                        init_sent = true;
                        fragment
                    }
                    Piece::Fragment(fragment) => {
                        if first_frame.is_none() {
                            first_frame = Some(queued.elapsed().as_millis() as u64);
                        }
                        fragment
                    }
                };
                sent += payload.len() as u64;
                let notes = side.lock().map(|mut side| side.take()).unwrap_or_default();
                for note in notes {
                    if stream.send(text_frame(note)).await.is_err() {
                        closed_by_client = true;
                        break 'relay;
                    }
                }
                if stream
                    .send(ws::Message::Binary(payload.to_vec()))
                    .await
                    .is_err()
                {
                    closed_by_client = true;
                    break 'relay;
                }
            }
            drop(output);
            pace.release();
            if let Some(child) = child.as_mut() {
                let _ = child.start_kill();
            }
            let mut pump = pump;
            let (written, capped) =
                match tokio::time::timeout(Duration::from_secs(2), &mut pump).await {
                    Ok(Ok(result)) => result,
                    _ => {
                        pump.abort();
                        let _ = pump.await;
                        (0, false)
                    }
                };
            if let Some(mut child) = child {
                let _ = child.wait().await;
            }
            if let Some(stderr) = stderr {
                if let Ok(tail) = stderr.await {
                    if !tail.trim().is_empty() {
                        tracing::warn!(detail = %tail.trim(), "playback ffmpeg reported");
                    }
                }
            }
            if !closed_by_client {
                let (notes, clocks, audio_records) = side
                    .lock()
                    .map(|mut side| (side.take(), side.clocks(), side.audio_records()))
                    .unwrap_or_default();
                for note in notes {
                    let _ = stream.send(text_frame(note)).await;
                }
                let _ = stream
                    .send(text_frame(serde_json::json!({
                        "type": "stats",
                        "mode": play.wire(),
                        "clocks": clocks,
                        "audioRecords": audio_records,
                        "init": init_sent,
                        "framesWritten": written,
                        "bytes": sent,
                        "firstFrameMs": first_frame,
                        "fps": fps.map(|value| (value * 1000.0).round() / 1000.0),
                        "fpsSettled": rate.settled(),
                        "codec": demuxer(codec),
                        "pipeline": if native { "native" } else { "ffmpeg" },
                        "windowSec": window,
                        "capped": capped,
                    })))
                    .await;
            }
            Ok(())
        })
    }))
}

#[cfg(test)]
mod tests {
    use crate::config::ModeStore;
    use proview_core::config::{Config, ProviderKind};
    use proview_core::provider::Mode;
    use rocket::http::Status;
    use rocket::local::blocking::Client;

    fn client() -> Client {
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
        Client::tracked(crate::app::build_with_modes(
            figment,
            config,
            ModeStore::new(Mode::Cloud, None),
        ))
        .expect("rocket")
    }

    #[test]
    fn the_file_list_refuses_a_request_without_a_session() {
        let rocket = client();
        let response = rocket
            .get("/api/playback/files?device=abc&channel=0&begin=2026-10-24%2000:00:00&end=2026-10-24%2023:59:59")
            .dispatch();
        assert_eq!(response.status(), Status::Unauthorized);
    }

    #[test]
    fn the_watch_socket_is_wired_and_refuses_a_plain_request() {
        let rocket = client();
        let response = rocket
            .get("/ws/playback?device=abc&name=/a.h264&begin=2026-10-24%2000:00:00&end=2026-10-24%2000:05:00")
            .dispatch();
        assert_eq!(response.status(), Status::BadRequest);
    }

    #[test]
    fn the_browser_script_is_served_next_to_the_app() {
        let rocket = client();
        let page = rocket.get("/").dispatch().into_string().expect("page");
        let start = page
            .find("/assets/index-")
            .expect("the page links its bundle");
        let end = start + page[start..].find(".js").expect("a script bundle") + 3;
        let response = rocket.get(page[start..end].to_string()).dispatch();
        assert_eq!(response.status(), Status::Ok);
        let body = response.into_string().expect("body");
        assert!(body.contains("/ws/playback"));
        assert!(body.contains("MediaSource"));
    }

    #[test]
    fn the_download_route_refuses_a_request_without_a_session() {
        let rocket = client();
        let response = rocket
            .get("/api/playback/download?device=abc&channel=0&name=/a.h264&begin=2026-10-24%2000:00:00&end=2026-10-24%2000:05:00&bytes=1024")
            .dispatch();
        assert_eq!(response.status(), Status::Unauthorized);
    }

    #[test]
    fn a_device_name_cannot_smuggle_a_header() {
        assert_eq!(
            super::download_name(
                "/a/b\"\r\nSet-Cookie: x=1; é.h264",
                "2026-10-24 10:34:00",
                "mp4"
            ),
            "2026-10-24_b___Set-Cookie__x_1___.mp4"
        );
        assert_eq!(
            super::download_name(
                "/idea0/2026-10-24/001/10.34.00-10.40.00[R][@e19][0].h264",
                "2026-10-24 10:34:00",
                "hevc"
            ),
            "2026-10-24_10.34.00-10.40.00[R][@e19][0].hevc"
        );
        assert_eq!(
            super::download_name("/", "2026-10-24 10:34:00", "mp4"),
            "2026-10-24_clip.mp4"
        );
        assert_eq!(
            super::download_name("/a.h264", "\"\r\nX: 1 10:00:00", "mp4"),
            "a.mp4"
        );
    }

    #[test]
    fn a_short_transfer_is_named_as_a_shortfall() {
        assert_eq!(
            super::short_fall(10, 20).as_deref(),
            Some("the device sent 10 of 20 bytes")
        );
        assert!(super::short_fall(20, 20).is_none());
        assert!(super::short_fall(21, 20).is_none());
    }

    #[test]
    fn a_download_declares_its_own_length() {
        use rocket::response::Responder;
        let rocket = client();
        let local = rocket.get("/");
        let request = local.inner();
        let response = super::Download {
            bytes: vec![1, 2, 3],
            filename: "clip.mp4".to_string(),
            kind: rocket::http::ContentType::MP4,
        }
        .respond_to(request)
        .expect("responder");
        assert_eq!(response.body().preset_size(), Some(3));
        assert_eq!(
            response.content_type(),
            Some(rocket::http::ContentType::MP4)
        );
        assert_eq!(
            response.headers().get_one("Content-Disposition"),
            Some("attachment; filename=\"clip.mp4\"")
        );
    }

    fn hevc_keyframe() -> Vec<u8> {
        let mut out = Vec::new();
        for nal in [
            "40010c01ffff0160000003000003000003000003003f2c0c00000fa00000bb8140",
            "4201010160000003000003000003000003003fa001202002887f9cbabc12e080",
            "4401c0f3c0e14524",
            "2601af10",
        ] {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend(
                (0..nal.len())
                    .step_by(2)
                    .map(|index| u8::from_str_radix(&nal[index..index + 2], 16).expect("hex")),
            );
        }
        out
    }

    fn record(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8, 0, 1, kind];
        match kind {
            0xfc => {
                out.extend_from_slice(&[0u8; 4]);
                out.extend_from_slice(&1u32.to_le_bytes());
                out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            }
            0xfd => out.extend_from_slice(&(body.len() as u32).to_le_bytes()),
            _ => {}
        }
        out.extend_from_slice(body);
        out
    }

    async fn feed(socket: &mut tokio::net::TcpStream, id: u16, payload: &[u8]) {
        use tokio::io::AsyncWriteExt;
        let header = proview_core::dvrip::frame::Header {
            session: 7,
            sequence: 0,
            channel: 0,
            end: false,
            id,
            length: payload.len() as u32,
        };
        socket.write_all(&header.encode()).await.expect("header");
        socket.write_all(payload).await.expect("body");
        socket.flush().await.expect("flush");
    }

    async fn device(frames: Vec<(u16, Vec<u8>)>, close: bool) -> tokio::net::TcpStream {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listen");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            for (id, payload) in frames {
                feed(&mut socket, id, &payload).await;
            }
            if close {
                drop(socket);
            } else {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
        tokio::net::TcpStream::connect(address)
            .await
            .expect("connect")
    }

    #[tokio::test]
    async fn a_complete_file_is_collected_to_its_payload() {
        let key = record(0xfc, &hevc_keyframe());
        let delta = record(0xfd, &[0, 0, 0, 1, 0x02, 0x01, 0xd0]);
        let wire = (key.len() + delta.len()) as u64;
        let socket = device(
            vec![
                (0x1234, b"not media".to_vec()),
                (0x058e, key.clone()),
                (0x0592, delta.clone()),
            ],
            false,
        )
        .await;
        let mut session = proview_core::dvrip::playback::Session::open(socket);
        let mut mux = super::ClipMux::new(wire as usize);
        super::collect(&mut session, wire, &mut mux)
            .await
            .expect("collect");
        let clip = mux.finish().expect("clip");
        assert!(clip.muxed);
        assert_eq!(clip.frames, 2);
        assert_eq!(&clip.bytes[4..8], b"ftyp");
    }

    #[tokio::test]
    async fn a_file_that_stops_before_its_last_byte_is_an_error() {
        let key = record(0xfc, &[3u8; 32]);
        let wire = key.len() as u64;
        let socket = device(vec![(0x058e, key)], true).await;
        let mut session = proview_core::dvrip::playback::Session::open(socket);
        let error = super::collect(&mut session, wire + 1, &mut super::ClipMux::new(0))
            .await
            .expect_err("a short file");
        assert!(error.contains("the tunnel broke"), "{error}");
    }

    #[tokio::test]
    async fn a_file_that_reports_more_than_it_holds_is_an_error() {
        let key = record(0xfc, &[4u8; 16]);
        let wire = key.len() as u64;
        let socket = device(vec![(0x058e, key)], true).await;
        let mut session = proview_core::dvrip::playback::Session::open(socket);
        let error = super::collect(&mut session, wire + 4096, &mut super::ClipMux::new(0))
            .await
            .expect_err("an overstated length");
        assert!(error.contains("the tunnel broke"), "{error}");
    }
}
