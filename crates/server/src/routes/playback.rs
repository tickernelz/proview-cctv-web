use std::process::Stdio;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use proview_core::cloud::envelope::{EnvelopeClient, Identity};
use proview_core::cloud::status::{auth_code_of, StatusClient, STATUS_HOST};
use proview_core::cloud::MISSING_CLOUD_IDENTITY;
use proview_core::config::Config;
use proview_core::dvrip::playback::{
    access_unit, codec_of, is_video, shift, snap_rate, FrameRate, MediaReader, PlaybackFile,
    RecordKind, Session, DOWNLOAD_DATA, PLAYBACK_DATA,
};
use proview_core::media::hevc;
use proview_core::media::Codec;
use proview_core::provider::{p2p, rps, VendorSession};
use proview_core::session::Store;
use proview_core::source::{Fmp4Splitter, Piece};
use rocket::http::{ContentType, CookieJar, Status};
use rocket::request::Request;
use rocket::response::{self, Responder, Response};
use rocket::serde::json::Json;
use rocket::State;
use rocket_ws as ws;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::routes::{fail, restore, session_id, Failure};

const PRIME_READ: Duration = Duration::from_secs(3);
const PRIME_WAIT: Duration = Duration::from_secs(8);
const DEFAULT_WINDOW: u32 = 60;
const MIN_WINDOW: u32 = 5;
const MAX_WINDOW: u32 = 600;
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
    pub begin: String,
    pub end: String,
    pub count: usize,
    pub bytes: u64,
    pub truncated: bool,
    pub cap: usize,
    pub elapsed_ms: u64,
    pub files: Vec<PlaybackFile>,
}

#[get("/playback/files?<device>&<channel>&<begin>&<end>&<kind>")]
#[allow(clippy::too_many_arguments)]
pub async fn files(
    device: String,
    channel: Option<u8>,
    begin: Option<String>,
    end: Option<String>,
    kind: Option<String>,
    cfg: &State<Config>,
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
    let started = Instant::now();
    let mut session = open_tunnel(cfg, &vendor, &device)
        .await
        .map_err(|message| fail(Status::BadGateway, message))?;
    let found = session
        .files(channel, &begin, &end, &kind)
        .await
        .map_err(|error| fail(Status::BadGateway, error.to_string()))?;
    let _ = session.logout().await;
    let bytes = found.iter().map(|file| file.bytes).sum();
    Ok(Json(FilesBody {
        ok: true,
        source: "cloud",
        device,
        channel,
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
}

impl<'r> Responder<'r, 'static> for Download {
    fn respond_to(self, _request: &'r Request<'_>) -> response::Result<'static> {
        Response::build()
            .header(ContentType::Binary)
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

async fn collect(session: &mut Session, listed: u64) -> Result<Vec<u8>, String> {
    let mut reader = MediaReader::new();
    let mut payload: Vec<u8> = Vec::new();
    let mut wire = 0u64;
    loop {
        while let Some(record) = reader.next_record() {
            if !is_video(&record) {
                continue;
            }
            if payload.len() as u64 + record.bytes.len() as u64 > DOWNLOAD_LIMIT {
                return Err(format!(
                    "the file is larger than the {DOWNLOAD_LIMIT} bytes this server holds"
                ));
            }
            payload.extend_from_slice(&record.bytes);
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
    Ok(payload)
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
    cfg: &State<Config>,
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
    let mut session = open_tunnel(cfg, &vendor, &device)
        .await
        .map_err(|message| fail(Status::BadGateway, message))?;
    session
        .claim(&file, channel)
        .await
        .map_err(|error| fail(Status::BadGateway, error.to_string()))?;
    session
        .download_start(&file, channel)
        .await
        .map_err(|error| fail(Status::BadGateway, error.to_string()))?;
    let collected = collect(&mut session, listed).await;
    let _ = session.download_stop(&file, channel).await;
    let _ = session.logout().await;
    let bytes = collected.map_err(|message| fail(Status::BadGateway, message))?;
    Ok(Download {
        bytes,
        filename: download_name(&name),
    })
}

fn download_name(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|letter| {
            if letter.is_ascii_alphanumeric() || matches!(letter, '.' | '_' | '-' | '@' | '[' | ']')
            {
                letter
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "clip.h264".to_string()
    } else {
        cleaned
    }
}

async fn open_tunnel(
    cfg: &Config,
    vendor: &VendorSession,
    serial: &str,
) -> Result<Session, String> {
    let token = vendor
        .token
        .as_deref()
        .ok_or("the session carries no cloud token; sign in again")?;
    let identity = Identity::resolve();
    if !identity.is_complete() {
        return Err(MISSING_CLOUD_IDENTITY.to_string());
    }
    let client = EnvelopeClient::with_base(identity, cfg.cloud_base.clone());
    let devices = client
        .device_list(token)
        .await
        .map_err(|error| error.to_string())?;
    let device = devices
        .iter()
        .find(|device| device.uuid == serial)
        .ok_or("the device is not on this account")?;
    let rps_id = device
        .extra
        .get("rps")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    if rps_id.is_empty() {
        return Err("the account device carries no rps id".to_string());
    }
    let status = StatusClient::new(STATUS_HOST)
        .query(serial, &auth_code_of(device))
        .await
        .map_err(|error| error.to_string())?;
    let (access_host, access_port) = status
        .access_server()
        .ok_or("no status cluster reported an access server")?;
    let session_id = rps::session_id();
    let client_token = rps::client_token();
    let body = p2p::agent_request(
        &rps_id,
        &client_token,
        serial,
        p2p::AGENT_SERVICE_MEDIA,
        &session_id,
        p2p::AGENT_STEP_MEDIA,
    );
    let agent = rps::AccessClient::new()
        .assign(&access_host, access_port, &body)
        .await
        .map_err(|error| error.to_string())?;
    let frame = p2p::handshake_frame(&rps_id, &client_token, serial, &session_id);
    let tunnel = rps::open(&agent, &frame)
        .await
        .map_err(|error| error.to_string())?;
    if !tunnel.reply.accepted() {
        return Err(format!(
            "the relay refused the tunnel with error {}",
            tunnel.reply.error_num
        ));
    }
    let mut session = Session::open(tunnel.stream);
    session
        .login(&cfg.device_user, &cfg.device_pass)
        .await
        .map_err(|error| error.to_string())?;
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

fn demuxer(codec: Codec) -> &'static str {
    match codec {
        Codec::H264 => "h264",
        Codec::H265 => "hevc",
    }
}

fn ffmpeg(codec: Codec, fps: Option<f64>) -> Command {
    let binary = std::env::var("PROVIEW_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string());
    let mut command = Command::new(binary);
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-flags",
        "low_delay",
        "-probesize",
        "32768",
        "-analyzeduration",
        "0",
        "-f",
        demuxer(codec),
    ]);
    if let Some(fps) = fps {
        command.args(["-framerate", &format!("{fps:.3}")]);
    }
    command.args([
        "-i",
        "pipe:0",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-tune",
        "zerolatency",
        "-profile:v",
        "baseline",
        "-g",
        "25",
        "-keyint_min",
        "25",
        "-sc_threshold",
        "0",
        "-an",
        "-f",
        "mp4",
        "-movflags",
        "+empty_moov+default_base_moof+frag_keyframe+separate_moof+omit_tfhd_offset",
        "pipe:1",
    ]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
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

#[get("/ws/playback?<device>&<channel>&<name>&<begin>&<end>&<start>&<window>")]
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
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<ws::Channel<'static>, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    if device.contains('/') || device.contains("..") {
        return Err(fail(Status::BadRequest, "invalid device id"));
    }
    let params = WatchQuery {
        device,
        channel,
        name,
        begin,
        end,
        start,
        window,
    };
    let config = cfg.inner().clone();
    Ok(ws.channel(move |mut stream| {
        Box::pin(async move {
            let channel = params.channel.unwrap_or(0);
            let from = params.start.unwrap_or(0);
            let window = params
                .window
                .unwrap_or(DEFAULT_WINDOW)
                .clamp(MIN_WINDOW, MAX_WINDOW);
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
            let mut session = match open_tunnel(&config, &vendor, &params.device).await {
                Ok(session) => session,
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
            let _ = stream
                .send(text_frame(serde_json::json!({
                    "type": "status",
                    "message": "asking the device for the clip",
                    "openMs": opened.elapsed().as_millis() as u64,
                })))
                .await;
            if let Err(error) = session.claim(&file, channel).await {
                let _ = stream
                    .send(text_frame(serde_json::json!({
                        "type": "error",
                        "message": error.to_string(),
                    })))
                    .await;
                return Ok(());
            }
            let queued = Instant::now();
            if let Err(error) = session.start(&file, channel).await {
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
            let mut child = match ffmpeg(codec, fps).spawn() {
                Ok(child) => child,
                Err(error) => {
                    let _ = stream
                        .send(text_frame(serde_json::json!({
                            "type": "error",
                            "message": format!("ffmpeg did not start: {error}"),
                        })))
                        .await;
                    return Ok(());
                }
            };
            let mut stdin = match child.stdin.take() {
                Some(stdin) => stdin,
                None => return Ok(()),
            };
            let mut stdout = match child.stdout.take() {
                Some(stdout) => stdout,
                None => return Ok(()),
            };
            let stderr = child.stderr.take().map(|mut handle| {
                tokio::spawn(async move {
                    let mut tail = String::new();
                    let mut chunk = [0u8; 2048];
                    while let Ok(read) = handle.read(&mut chunk).await {
                        if read == 0 {
                            break;
                        }
                        tail.push_str(&String::from_utf8_lossy(&chunk[..read]));
                        if tail.len() > 4096 {
                            tail = tail.chars().skip(tail.chars().count() - 2048).collect();
                        }
                    }
                    tail
                })
            });
            let _ = stream
                .send(text_frame(serde_json::json!({
                    "type": "status",
                    "message": "transcoding the clip",
                    "codec": demuxer(codec),
                    "fps": fps.map(|value| (value * 1000.0).round() / 1000.0),
                    "fpsSettled": rate.settled(),
                    "fpsSource": fps_source,
                    "fpsMeasured": rate.estimate().map(|value| (value * 1000.0).round() / 1000.0),
                    "keySpan": rate.span_seconds(),
                    "primedFrames": video,
                    "primedKeys": key_frames,
                    "primeMs": queued.elapsed().as_millis() as u64,
                    "windowSec": window,
                })))
                .await;

            let primed = pending;
            let stop_file = file.clone();
            let window_frames = (f64::from(window) * fps.unwrap_or(25.0)).ceil() as usize;
            let pump = tokio::spawn(async move {
                for frame in primed {
                    if stdin.write_all(&frame).await.is_err() {
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
                        let mut bytes = Vec::new();
                        if annex_b(&frame, &mut bytes) {
                            written += 1;
                            if stdin.write_all(&bytes).await.is_err() {
                                keep = false;
                                break;
                            }
                        }
                    }
                    if !keep {
                        break;
                    }
                }
                let _ = stdin.shutdown().await;
                let _ = session.stop(&stop_file, channel).await;
                (written, capped)
            });

            let mut splitter = Fmp4Splitter::new();
            let mut buffer = vec![0u8; 64 * 1024];
            let mut init_sent = false;
            let mut sent = 0u64;
            let mut first_frame: Option<u64> = None;
            let mut closed_by_client = false;
            'relay: loop {
                let read = tokio::select! {
                    biased;
                    incoming = stream.next() => {
                        match incoming {
                            None | Some(Err(_)) | Some(Ok(ws::Message::Close(_))) => {
                                closed_by_client = true;
                                break 'relay;
                            }
                            Some(Ok(_)) => continue,
                        }
                    }
                    read = stdout.read(&mut buffer) => match read {
                        Ok(0) => break,
                        Ok(read) => read,
                        Err(_) => break,
                    },
                };
                for piece in splitter.push(&buffer[..read]) {
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
                    if stream
                        .send(ws::Message::Binary(payload.to_vec()))
                        .await
                        .is_err()
                    {
                        closed_by_client = true;
                        break 'relay;
                    }
                }
            }
            drop(stdout);
            let _ = child.start_kill();
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
            let _ = child.wait().await;
            if let Some(stderr) = stderr {
                if let Ok(tail) = stderr.await {
                    if !tail.trim().is_empty() {
                        tracing::warn!(detail = %tail.trim(), "playback ffmpeg reported");
                    }
                }
            }
            if !closed_by_client {
                let _ = stream
                    .send(text_frame(serde_json::json!({
                        "type": "stats",
                        "init": init_sent,
                        "framesWritten": written,
                        "bytes": sent,
                        "firstFrameMs": first_frame,
                        "fps": fps.map(|value| (value * 1000.0).round() / 1000.0),
                        "fpsSettled": rate.settled(),
                        "codec": demuxer(codec),
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
            super::download_name("/a/b\"\r\nSet-Cookie: x=1; é.h264"),
            "b___Set-Cookie__x_1___.h264"
        );
        assert_eq!(
            super::download_name("/idea0/2026-10-24/001/10.34.00-10.40.00[R][@e19][0].h264"),
            "10.34.00-10.40.00[R][@e19][0].h264"
        );
        assert_eq!(super::download_name("/"), "clip.h264");
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
            filename: "clip.h264".to_string(),
        }
        .respond_to(request)
        .expect("responder");
        assert_eq!(response.body().preset_size(), Some(3));
        assert_eq!(
            response.headers().get_one("Content-Disposition"),
            Some("attachment; filename=\"clip.h264\"")
        );
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
        let key = record(0xfc, &[1u8; 40]);
        let delta = record(0xfd, &[2u8; 24]);
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
        let payload = super::collect(&mut session, wire).await.expect("collect");
        assert_eq!(payload, [vec![1u8; 40], vec![2u8; 24]].concat());
    }

    #[tokio::test]
    async fn a_file_that_stops_before_its_last_byte_is_an_error() {
        let key = record(0xfc, &[3u8; 32]);
        let wire = key.len() as u64;
        let socket = device(vec![(0x058e, key)], true).await;
        let mut session = proview_core::dvrip::playback::Session::open(socket);
        let error = super::collect(&mut session, wire + 1)
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
        let error = super::collect(&mut session, wire + 4096)
            .await
            .expect_err("an overstated length");
        assert!(error.contains("the tunnel broke"), "{error}");
    }
}
