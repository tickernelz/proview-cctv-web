use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

use crate::live::{Fragment, LiveAudio, LiveHub, LiveMode, LivePacket, StreamKey, StreamMeta};
use crate::media::fmp4::{init_segment, media_segment, Sample, TrackConfig};
use crate::media::mux::{self, FragmentMux, VideoConfig};
use crate::media::{h264, hevc, rtp, Codec};
use crate::rtsp::{RtspSession, Track};

#[derive(Debug, Clone)]
pub struct RtspTarget {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub user: String,
    pub password: String,
}

impl RtspTarget {
    pub fn from_host_port(host: &str, port: u16) -> Self {
        Self {
            host: host.to_string(),
            port,
            path: "/".to_string(),
            user: String::new(),
            password: String::new(),
        }
    }

    pub fn with_credentials(mut self, user: &str, password: &str) -> Self {
        self.user = user.to_string();
        self.password = password.to_string();
        self
    }

    pub fn with_path(mut self, path: &str) -> Self {
        self.path = if path.is_empty() {
            "/".to_string()
        } else {
            path.to_string()
        };
        self
    }
}

#[derive(Debug)]
pub enum SourceError {
    Rtsp(crate::rtsp::RtspError),
    Io(std::io::Error),
    Ffmpeg(String),
    NoVideoTrack,
    FfmpegMissing,
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Rtsp(error) => write!(f, "rtsp: {error}"),
            SourceError::Io(error) => write!(f, "io: {error}"),
            SourceError::Ffmpeg(message) => write!(f, "ffmpeg: {message}"),
            SourceError::NoVideoTrack => write!(f, "sdp has no video track"),
            SourceError::FfmpegMissing => write!(
                f,
                "ffmpeg is not installed; add its path under Settings, Configuration, Streaming, or install ffmpeg for HEVC cameras"
            ),
        }
    }
}

impl std::error::Error for SourceError {}

pub enum Piece {
    Init(Fragment),
    Fragment(Fragment),
}

#[derive(Default)]
pub struct Fmp4Splitter {
    buffer: Vec<u8>,
    init: Vec<u8>,
    init_complete: bool,
    fragment: Vec<u8>,
    seen_moof: bool,
}

impl Fmp4Splitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<Piece> {
        self.buffer.extend_from_slice(chunk);
        let mut pieces = Vec::new();
        loop {
            if self.buffer.len() < 8 {
                break;
            }
            let mut size = u32::from_be_bytes([
                self.buffer[0],
                self.buffer[1],
                self.buffer[2],
                self.buffer[3],
            ]) as usize;
            let mut header = 8;
            if size == 1 {
                if self.buffer.len() < 16 {
                    break;
                }
                let mut largesize = [0u8; 8];
                largesize.copy_from_slice(&self.buffer[8..16]);
                size = u64::from_be_bytes(largesize) as usize;
                header = 16;
            }
            if size < header || self.buffer.len() < size {
                break;
            }
            let kind: [u8; 4] = [
                self.buffer[4],
                self.buffer[5],
                self.buffer[6],
                self.buffer[7],
            ];
            let body: Vec<u8> = self.buffer.drain(..size).collect();
            if !self.init_complete {
                if &kind == b"ftyp" || &kind == b"moov" {
                    self.init.extend_from_slice(&body);
                    if &kind == b"moov" {
                        self.init_complete = true;
                        pieces.push(Piece::Init(Bytes::from(std::mem::take(&mut self.init))));
                    }
                }
                continue;
            }
            match &kind {
                b"moof" => {
                    self.fragment.extend_from_slice(&body);
                    self.seen_moof = true;
                }
                b"mdat" if self.seen_moof => {
                    self.fragment.extend_from_slice(&body);
                    self.seen_moof = false;
                    pieces.push(Piece::Fragment(Bytes::from(std::mem::take(
                        &mut self.fragment,
                    ))));
                }
                b"mfra" | b"sidx" => {}
                _ => {
                    if self.seen_moof {
                        self.fragment.extend_from_slice(&body);
                    }
                }
            }
        }
        pieces
    }
}

pub fn ffmpeg_command() -> Command {
    let binary = std::env::var("PROVIEW_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string());
    let mut command = Command::new(binary);
    crate::process::hide_console(command.as_std_mut());
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
        "hevc",
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
    ]);
    command.args([
        "-an",
        "-f",
        "mp4",
        "-movflags",
        "+empty_moov+default_base_moof+frag_keyframe+separate_moof+omit_tfhd_offset",
        "pipe:1",
    ]);
    command
}

pub async fn drain_stderr(mut child: Child) -> Option<String> {
    let mut stderr = child.stderr.take()?;
    let mut buffer = Vec::new();
    let _ = stderr.read_to_end(&mut buffer).await;
    let text = String::from_utf8_lossy(&buffer).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

pub async fn pump(hub: Arc<LiveHub>, key: StreamKey, target: RtspTarget, stop: Arc<AtomicBool>) {
    tracing::info!(device = %key.device, mode = key.mode.as_str(), host = %target.host, "live source starting");
    match pump_inner(&hub, &key, target, &stop).await {
        Ok(()) => {
            tracing::info!(device = %key.device, mode = key.mode.as_str(), "live source finished")
        }
        Err(error) => {
            tracing::warn!(%error, device = %key.device, mode = key.mode.as_str(), "live source stopped")
        }
    }
}

fn audio_clock(codec: &str) -> Option<u32> {
    match codec {
        "PCMA" | "PCMU" => Some(8000),
        _ => None,
    }
}

async fn pump_inner(
    hub: &Arc<LiveHub>,
    key: &StreamKey,
    target: RtspTarget,
    stop: &Arc<AtomicBool>,
) -> Result<(), SourceError> {
    let mut session = RtspSession::open(
        &target.host,
        target.port,
        &target.path,
        &target.user,
        &target.password,
    )
    .await
    .map_err(SourceError::Rtsp)?;
    let video = session
        .video_track()
        .cloned()
        .ok_or(SourceError::NoVideoTrack)?;
    let audio = session.audio_track().cloned();
    session
        .setup_video((0, 1))
        .await
        .map_err(SourceError::Rtsp)?;
    let audio_channel = match &audio {
        Some(track) => match session.setup_audio((2, 3)).await {
            Ok(pair) => {
                tracing::info!(codec = %track.codec, channel = pair.0, "audio track set up");
                Some(pair.0)
            }
            Err(error) => {
                tracing::warn!(%error, codec = %track.codec, "audio setup refused");
                None
            }
        },
        None => None,
    };
    session.play().await.map_err(SourceError::Rtsp)?;

    let mut meta = StreamMeta {
        video_codec: Some(video.codec.clone()),
        audio_codec: audio.as_ref().map(|track| track.codec.clone()),
        audio_clock: audio.as_ref().and_then(|track| audio_clock(&track.codec)),
        ..StreamMeta::default()
    };
    let announced = match (&video.vps, &video.sps, &video.pps) {
        (Some(vps), Some(sps), Some(pps)) => VideoConfig::hevc(vps, sps, pps).ok(),
        _ => None,
    };
    if let Some(config) = &announced {
        meta.width = Some(config.width);
        meta.height = Some(config.height);
        meta.codec_string = Some(config.codec_string.clone());
    }
    let route = live_route(key.mode, &video.codec, announced.as_ref());
    if key.mode == LiveMode::Passthrough && route != LiveRoute::Passthrough {
        meta.codec_string = None;
    }
    hub.set_meta(key, meta.clone());

    match route {
        LiveRoute::Passthrough => {
            let handover =
                pump_passthrough(hub, key, session, &video, audio_channel, meta.clone(), stop)
                    .await?;
            match handover {
                Some(session) => {
                    meta.codec_string = None;
                    hub.set_meta(key, meta);
                    pump_transcode(hub, key, session, video, audio_channel, stop).await
                }
                None => Ok(()),
            }
        }
        LiveRoute::NativeH264 => {
            pump_h264(hub, key, session, video, audio_channel, meta, stop).await
        }
        LiveRoute::Ffmpeg => pump_transcode(hub, key, session, video, audio_channel, stop).await,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveRoute {
    Passthrough,
    NativeH264,
    Ffmpeg,
}

fn live_route(mode: LiveMode, codec: &str, announced: Option<&VideoConfig>) -> LiveRoute {
    let hevc = codec.eq_ignore_ascii_case("H265");
    if mode == LiveMode::Passthrough && hevc && !announced.is_some_and(|config| config.reorders) {
        return LiveRoute::Passthrough;
    }
    match transcode_path(codec) {
        TranscodePath::NativeH264 => LiveRoute::NativeH264,
        TranscodePath::Ffmpeg => LiveRoute::Ffmpeg,
    }
}

async fn pump_transcode(
    hub: &Arc<LiveHub>,
    key: &StreamKey,
    mut session: RtspSession,
    video: Track,
    audio_channel: Option<u8>,
    stop: &Arc<AtomicBool>,
) -> Result<(), SourceError> {
    let parameter_sets: Vec<Vec<u8>> = [video.vps.clone(), video.sps.clone(), video.pps.clone()]
        .into_iter()
        .flatten()
        .collect();
    let (mut stdin, reader, child) = spawn_fmp4_publisher(hub, key, &parameter_sets).await?;

    let mut depacketizer = hevc::Depacketizer::new();
    let mut written = Vec::with_capacity(256 * 1024);
    let result = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        let (channel, packet) =
            match tokio::time::timeout(Duration::from_secs(10), session.next_packet()).await {
                Ok(Ok(packet)) => packet,
                Ok(Err(error)) => break Err(SourceError::Rtsp(error)),
                Err(_) => continue,
            };
        if Some(channel) == audio_channel {
            forward_audio(hub, key, &packet);
            continue;
        }
        let Some(parsed) = rtp::parse(&packet) else {
            continue;
        };
        if parsed.payload_type != video.payload_type {
            continue;
        }
        written.clear();
        for nal in depacketizer.push(&parsed) {
            hevc::append_annexb(&nal, &mut written);
        }
        if !written.is_empty() && stdin.write_all(&written).await.is_err() {
            break Ok(());
        }
    };

    drop(stdin);
    hub.publisher_done(key);
    let stderr = drain_stderr(child).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), session.teardown()).await;
    let _ = tokio::time::timeout(Duration::from_secs(3), reader).await;
    if let Some(text) = stderr {
        tracing::debug!(%text, "ffmpeg stderr");
    }
    result
}

pub const RTP_PCMA: u8 = 8;

pub fn rtsp_audio(packet: &[u8]) -> Option<LiveAudio> {
    let parsed = rtp::parse(packet)?;
    if parsed.payload_type != RTP_PCMA || parsed.payload.is_empty() {
        return None;
    }
    Some(LiveAudio {
        codec: "alaw",
        rate: 8000,
        frame: u64::from(parsed.sequence),
        bytes: parsed.payload.to_vec(),
    })
}

fn forward_audio(hub: &LiveHub, key: &StreamKey, packet: &[u8]) {
    match rtsp_audio(packet) {
        Some(audio) => hub.publish_audio(key, audio),
        None => hub.note_audio(key),
    }
}

fn h264_codec_string(sps: &[u8]) -> String {
    let profile = sps.get(1).copied().unwrap_or(0x42);
    let compatibility = sps.get(2).copied().unwrap_or(0x00);
    let level = sps.get(3).copied().unwrap_or(0x1e);
    format!("avc1.{profile:02X}{compatibility:02X}{level:02X}")
}

fn h264_init(sps: &[u8], pps: &[u8]) -> Option<(Fragment, String, u32, u32)> {
    if pps.is_empty() || h264::nal_type(sps) != Some(7) || h264::nal_type(pps) != Some(8) {
        return None;
    }
    let info = h264::parse_sps(sps).ok()?;
    let track = TrackConfig {
        width: info.width.max(16),
        height: info.height.max(16),
        sps: sps.to_vec(),
        pps: pps.to_vec(),
    };
    Some((
        Fragment::from(init_segment(&track)),
        h264_codec_string(sps),
        track.width,
        track.height,
    ))
}

#[derive(Default)]
struct H264Mux {
    sequence: u32,
    base: u64,
    previous_pts: Option<u64>,
    samples: Vec<Sample>,
}

const FRAGMENT_FRAMES: usize = 10;

impl H264Mux {
    fn new() -> Self {
        Self {
            sequence: 1,
            ..Self::default()
        }
    }

    fn push(&mut self, pts_us: u64, keyframe: bool, nals: &[Vec<u8>]) -> Option<(Fragment, bool)> {
        let full = self.samples.len() >= FRAGMENT_FRAMES;
        let flushed = if !self.samples.is_empty() && (keyframe || full) {
            self.flush()
        } else {
            None
        };
        let duration = match self.previous_pts {
            Some(previous) => {
                let delta = pts_us.saturating_sub(previous);
                ((delta * 90) / 1_000).clamp(1, 90_000) as u32
            }
            None => 3_600,
        };
        if self.previous_pts.is_none() {
            self.base = (pts_us * 90) / 1_000;
        }
        self.previous_pts = Some(pts_us);
        self.samples.push(Sample {
            data: h264::avcc(nals),
            duration,
            keyframe,
        });
        flushed
    }

    fn flush(&mut self) -> Option<(Fragment, bool)> {
        if self.samples.is_empty() {
            return None;
        }
        let samples = std::mem::take(&mut self.samples);
        let keyframe = samples
            .first()
            .map(|sample| sample.keyframe)
            .unwrap_or(false);
        let segment = media_segment(self.sequence, self.base, &samples);
        self.base += samples
            .iter()
            .map(|sample| u64::from(sample.duration))
            .sum::<u64>();
        self.sequence = self.sequence.wrapping_add(1);
        Some((Fragment::from(segment), keyframe))
    }
}

#[allow(clippy::too_many_arguments)]
async fn pump_h264(
    hub: &Arc<LiveHub>,
    key: &StreamKey,
    mut session: RtspSession,
    video: Track,
    audio_channel: Option<u8>,
    mut meta: StreamMeta,
    stop: &Arc<AtomicBool>,
) -> Result<(), SourceError> {
    let _sender = hub.publisher(key.clone());
    let mut reader = h264::Reader::new();
    let mut gate = Gate::default();
    let mut sps = video.sps.clone();
    let mut pps = video.pps.clone();
    let mut ready = false;
    let mut pending: Vec<Vec<u8>> = Vec::new();
    let mut mux = H264Mux::new();
    let mut units = 0u64;
    let result = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        let (channel, packet) =
            match tokio::time::timeout(Duration::from_secs(10), session.next_packet()).await {
                Ok(Ok(packet)) => packet,
                Ok(Err(error)) => break Err(SourceError::Rtsp(error)),
                Err(_) => continue,
            };
        if Some(channel) == audio_channel {
            forward_audio(hub, key, &packet);
            continue;
        }
        let Some(parsed) = rtp::parse(&packet) else {
            continue;
        };
        if parsed.payload_type != video.payload_type {
            continue;
        }
        let Some(unit) = reader.push(&parsed) else {
            continue;
        };
        for nal in &unit.nals {
            match h264::nal_type(nal) {
                Some(7) => sps = Some(nal.clone()),
                Some(8) => pps = Some(nal.clone()),
                _ => {}
            }
        }
        if !ready {
            if let (Some(sps_nal), Some(pps_nal)) = (sps.as_deref(), pps.as_deref()) {
                if let Some((init, codec, width, height)) = h264_init(sps_nal, pps_nal) {
                    meta.codec_string = Some(codec);
                    meta.width = Some(width);
                    meta.height = Some(height);
                    hub.set_meta(key, meta.clone());
                    hub.publish_packet(key, LivePacket::init(init));
                    ready = true;
                }
            }
        }
        if !ready || !gate.accept(unit.keyframe) {
            pending.clear();
            hub.note_drop(key);
            continue;
        }
        let mut nals = std::mem::take(&mut pending);
        nals.extend(unit.nals);
        if !nals.iter().any(|nal| h264::is_vcl(nal)) {
            pending = nals;
            continue;
        }
        units += 1;
        if let Some((fragment, keyframe)) = mux.push(unit.pts_us, unit.keyframe, &nals) {
            hub.publish_media(key, fragment, keyframe);
        }
    };

    if let Some((fragment, keyframe)) = mux.flush() {
        hub.publish_media(key, fragment, keyframe);
    }
    tracing::info!(units, dropped = reader.dropped(), "h264 source finished");
    hub.publisher_done(key);
    let _ = tokio::time::timeout(Duration::from_secs(2), session.teardown()).await;
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscodePath {
    NativeH264,
    Ffmpeg,
}

fn transcode_path(codec: &str) -> TranscodePath {
    if codec.eq_ignore_ascii_case("H264") || codec.eq_ignore_ascii_case("AVC") {
        TranscodePath::NativeH264
    } else {
        TranscodePath::Ffmpeg
    }
}

#[derive(Default)]
struct Gate {
    started: bool,
}

impl Gate {
    fn accept(&mut self, keyframe: bool) -> bool {
        if keyframe {
            self.started = true;
        }
        self.started
    }
}

pub struct HevcPassthrough {
    config: Option<VideoConfig>,
    mux: FragmentMux,
    previous_pts: Option<u64>,
    started: bool,
}

impl Default for HevcPassthrough {
    fn default() -> Self {
        Self::new()
    }
}

impl HevcPassthrough {
    pub fn new() -> Self {
        Self {
            config: None,
            mux: FragmentMux::new(1),
            previous_pts: None,
            started: false,
        }
    }

    pub fn with_config(config: VideoConfig) -> Self {
        Self {
            config: Some(config),
            ..Self::new()
        }
    }

    pub fn config(&self) -> Option<&VideoConfig> {
        self.config.as_ref()
    }

    pub fn learn(&mut self, nals: &[Vec<u8>]) -> Option<&VideoConfig> {
        if self.config.is_some() {
            return None;
        }
        let built = VideoConfig::from_nals(Codec::H265, nals).ok()?;
        self.config = Some(built);
        self.config.as_ref()
    }

    pub fn push(
        &mut self,
        nals: &[Vec<u8>],
        keyframe: bool,
        duration: u32,
    ) -> Vec<(Fragment, bool)> {
        if self.config.is_none() {
            return Vec::new();
        }
        if keyframe {
            self.started = true;
        }
        if !self.started {
            return Vec::new();
        }
        let data = mux::sample_data(Codec::H265, nals);
        if data.is_empty() {
            return Vec::new();
        }
        self.mux
            .push(Sample {
                data,
                duration,
                keyframe,
            })
            .into_iter()
            .map(|(segment, keyframe)| (Fragment::from(segment), keyframe))
            .collect()
    }

    pub fn duration_since(&mut self, pts_us: u64) -> u32 {
        let duration = match self.previous_pts {
            Some(previous) => {
                ((pts_us.saturating_sub(previous) * 90) / 1_000).clamp(1, 90_000) as u32
            }
            None => 3_600,
        };
        self.previous_pts = Some(pts_us);
        duration
    }

    pub fn flush(&mut self) -> Option<(Fragment, bool)> {
        self.mux
            .flush()
            .map(|(segment, keyframe)| (Fragment::from(segment), keyframe))
    }
}

pub fn publish_config(hub: &LiveHub, key: &StreamKey, meta: &mut StreamMeta, config: &VideoConfig) {
    meta.codec_string = Some(config.codec_string.clone());
    meta.width = Some(config.width);
    meta.height = Some(config.height);
    hub.set_meta(key, meta.clone());
    hub.publish_packet(
        key,
        LivePacket::init(Fragment::from(config.fragmented_init())),
    );
}

#[allow(clippy::too_many_arguments)]
async fn pump_passthrough(
    hub: &Arc<LiveHub>,
    key: &StreamKey,
    mut session: RtspSession,
    video: &Track,
    audio_channel: Option<u8>,
    mut meta: StreamMeta,
    stop: &Arc<AtomicBool>,
) -> Result<Option<RtspSession>, SourceError> {
    let mut passthrough = match (&video.vps, &video.sps, &video.pps) {
        (Some(vps), Some(sps), Some(pps)) => VideoConfig::hevc(vps, sps, pps)
            .map(HevcPassthrough::with_config)
            .unwrap_or_default(),
        _ => HevcPassthrough::new(),
    };
    let _sender = hub.publisher(key.clone());
    if let Some(config) = passthrough.config() {
        publish_config(hub, key, &mut meta, config);
    }

    let mut reader = hevc::Reader::new();
    let mut units = 0u64;
    let mut handover = false;
    let result = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        let (channel, packet) =
            match tokio::time::timeout(Duration::from_secs(10), session.next_packet()).await {
                Ok(Ok(packet)) => packet,
                Ok(Err(error)) => break Err(SourceError::Rtsp(error)),
                Err(_) => continue,
            };
        if Some(channel) == audio_channel {
            forward_audio(hub, key, &packet);
            continue;
        }
        let Some(parsed) = rtp::parse(&packet) else {
            continue;
        };
        if parsed.payload_type != video.payload_type {
            continue;
        }
        let Some(unit) = reader.push(&parsed) else {
            continue;
        };
        if let Some(config) = passthrough.learn(&unit.nals) {
            if config.reorders {
                handover = true;
                break Ok(());
            }
            let config = config.clone();
            publish_config(hub, key, &mut meta, &config);
        }
        let duration = passthrough.duration_since(unit.pts_us);
        let fragments = passthrough.push(&unit.nals, unit.keyframe, duration);
        if fragments.is_empty() {
            hub.note_drop(key);
            continue;
        }
        units += 1;
        for (fragment, keyframe) in fragments {
            hub.publish_media(key, fragment, keyframe);
        }
    };
    if handover {
        tracing::info!("the stream reorders frames, so it is transcoded instead");
        return result.map(|()| Some(session));
    }
    if let Some((fragment, keyframe)) = passthrough.flush() {
        hub.publish_media(key, fragment, keyframe);
    }

    tracing::info!(
        units,
        dropped = reader.dropped(),
        "passthrough source finished"
    );
    hub.publisher_done(key);
    let _ = tokio::time::timeout(Duration::from_secs(2), session.teardown()).await;
    result.map(|()| None)
}

pub type Fmp4Publisher = (
    tokio::process::ChildStdin,
    tokio::task::JoinHandle<()>,
    Child,
);

pub async fn spawn_fmp4_publisher(
    hub: &Arc<LiveHub>,
    key: &StreamKey,
    parameter_sets: &[Vec<u8>],
) -> Result<Fmp4Publisher, SourceError> {
    let mut child = ffmpeg_command()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SourceError::FfmpegMissing
            } else {
                SourceError::Io(error)
            }
        })?;
    let mut stdin = child.stdin.take().ok_or(SourceError::FfmpegMissing)?;
    let mut stdout = child.stdout.take().ok_or(SourceError::FfmpegMissing)?;
    let mut head = Vec::new();
    for parameter in parameter_sets {
        hevc::append_annexb(parameter, &mut head);
    }
    if !head.is_empty() {
        stdin.write_all(&head).await.map_err(SourceError::Io)?;
    }
    let _sender = hub.publisher(key.clone());
    let publish_hub = hub.clone();
    let publish_key = key.clone();
    let reader = tokio::spawn(async move {
        let mut splitter = Fmp4Splitter::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let read = match stdout.read(&mut chunk).await {
                Ok(0) => break,
                Ok(read) => read,
                Err(_) => break,
            };
            for piece in splitter.push(&chunk[..read]) {
                match piece {
                    Piece::Init(bytes) => {
                        publish_hub.publish_packet(&publish_key, LivePacket::init(bytes));
                    }
                    Piece::Fragment(bytes) => {
                        publish_hub.publish(&publish_key, bytes);
                    }
                }
            }
        }
    });
    Ok((stdin, reader, child))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxes(parts: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (kind, body) in parts {
            out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
            out.extend_from_slice(&kind[..]);
            out.extend_from_slice(body);
        }
        out
    }

    #[test]
    fn init_is_emitted_once_and_then_one_piece_per_moof_and_mdat_pair() {
        let mut splitter = Fmp4Splitter::new();
        let init = boxes(&[(b"ftyp", b"isom"), (b"moov", b"trak")]);
        let first = boxes(&[(b"moof", b"mfhd"), (b"mdat", b"frame-one")]);
        let second = boxes(&[(b"moof", b"mfhd"), (b"mdat", b"frame-two")]);
        let mut stream = init.clone();
        stream.extend_from_slice(&first);
        stream.extend_from_slice(&second);
        let mut pieces = Vec::new();
        for chunk in stream.chunks(7) {
            pieces.extend(splitter.push(chunk));
        }
        assert_eq!(pieces.len(), 3);
        match &pieces[0] {
            Piece::Init(bytes) => assert_eq!(bytes.as_ref(), init.as_slice()),
            Piece::Fragment(_) => panic!("first piece must be the init segment"),
        }
        match (&pieces[1], &pieces[2]) {
            (Piece::Fragment(one), Piece::Fragment(two)) => {
                assert_eq!(one.as_ref(), first.as_slice());
                assert_eq!(two.as_ref(), second.as_slice());
            }
            _ => panic!("expected two media fragments"),
        }
    }

    #[test]
    fn an_incomplete_box_waits_for_more_bytes() {
        let mut splitter = Fmp4Splitter::new();
        let init = boxes(&[(b"ftyp", b"isom"), (b"moov", b"trak")]);
        assert!(splitter.push(&init[..4]).is_empty());
        let pieces = splitter.push(&init[4..]);
        assert_eq!(pieces.len(), 1);
    }

    #[test]
    fn ffmpeg_is_only_asked_to_transcode_hevc() {
        let command = ffmpeg_command();
        let args: Vec<String> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert!(args.windows(2).any(|pair| pair == ["-f", "hevc"]));
        assert!(args.contains(&"libx264".to_string()));
        assert!(args.contains(&"pipe:1".to_string()));
        assert!(!args.contains(&"copy".to_string()));
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
            .collect()
    }

    #[test]
    fn a_passthrough_request_follows_what_the_camera_can_send() {
        let camera = VideoConfig::hevc(
            &hex("40010c01ffff0160000003000003000003000003003f2c0c00000fa00000bb8140"),
            &hex("4201010160000003000003000003000003003fa001202002887f9cbabc12e080"),
            &hex("4401c0f3c0e14524"),
        )
        .expect("camera");
        let b_frames = VideoConfig::hevc(
            &hex("40010c01ffff01600000030090000003000003003c959809"),
            &hex("42010101600000030090000003000003003ca00a080f165959a4932bc05a020000030002000003003210"),
            &hex("4401c172b46240"),
        )
        .expect("x265 bframes=4");
        let passthrough = LiveMode::Passthrough;
        assert_eq!(
            live_route(passthrough, "H265", Some(&camera)),
            LiveRoute::Passthrough
        );
        assert_eq!(
            live_route(passthrough, "H265", None),
            LiveRoute::Passthrough
        );
        assert_eq!(
            live_route(passthrough, "H265", Some(&b_frames)),
            LiveRoute::Ffmpeg
        );
        assert_eq!(live_route(passthrough, "H264", None), LiveRoute::NativeH264);
        assert_eq!(live_route(passthrough, "MJPEG", None), LiveRoute::Ffmpeg);
        assert_eq!(
            live_route(LiveMode::Transcode, "H265", Some(&camera)),
            LiveRoute::Ffmpeg
        );
        assert_eq!(
            live_route(LiveMode::Transcode, "H264", None),
            LiveRoute::NativeH264
        );
    }

    #[test]
    fn h264_cameras_use_the_native_remux_not_ffmpeg() {
        assert_eq!(transcode_path("H264"), TranscodePath::NativeH264);
        assert_eq!(transcode_path("h264"), TranscodePath::NativeH264);
        assert_eq!(transcode_path("AVC"), TranscodePath::NativeH264);
        assert_eq!(transcode_path("H265"), TranscodePath::Ffmpeg);
        assert_eq!(transcode_path("MJPEG"), TranscodePath::Ffmpeg);
    }

    fn rtp_packet(payload_type: u8, sequence: u16, payload: &[u8]) -> Vec<u8> {
        let mut packet = vec![0x80, payload_type, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        packet[2..4].copy_from_slice(&sequence.to_be_bytes());
        packet.extend_from_slice(payload);
        packet
    }

    #[tokio::test]
    async fn rtsp_pcma_reaches_the_live_audio_channel_and_other_codecs_are_only_counted() {
        let audio = rtsp_audio(&rtp_packet(RTP_PCMA, 7, &[0xd5; 160])).expect("pcma");
        assert_eq!(audio.codec, "alaw");
        assert_eq!(audio.rate, 8000);
        assert_eq!(audio.frame, 7);
        assert_eq!(audio.bytes, vec![0xd5u8; 160]);
        assert!(rtsp_audio(&rtp_packet(0, 8, &[0xff; 160])).is_none());
        assert!(rtsp_audio(&rtp_packet(RTP_PCMA, 9, &[])).is_none());

        let hub = LiveHub::new(8);
        let key = StreamKey::new("dev", 1, false, LiveMode::Passthrough);
        let mut subscription = hub.subscribe(key.clone());
        hub.set_meta(
            &key,
            StreamMeta {
                audio_codec: Some("PCMA".to_string()),
                audio_clock: Some(8000),
                ..StreamMeta::default()
            },
        );
        forward_audio(&hub, &key, &rtp_packet(RTP_PCMA, 1, &[0x55; 160]));
        forward_audio(&hub, &key, &rtp_packet(0, 2, &[0xff; 160]));
        let heard = subscription.audio.try_recv().expect("forwarded");
        assert_eq!(heard.bytes, vec![0x55u8; 160]);
        assert!(subscription.audio.try_recv().is_err());
        assert_eq!(hub.snapshots()[0].audio_packets, 2);
        assert_eq!(
            hub.meta(&key).and_then(|meta| meta.audio_codec).as_deref(),
            Some("PCMA")
        );
        hub.release(&key);
    }

    #[test]
    fn audio_clock_is_known_for_the_g711_codecs() {
        assert_eq!(audio_clock("PCMA"), Some(8000));
        assert_eq!(audio_clock("PCMU"), Some(8000));
        assert_eq!(audio_clock("AAC"), None);
    }

    #[test]
    fn the_gate_drops_deltas_until_the_first_keyframe() {
        let mut gate = Gate::default();
        assert!(!gate.accept(false));
        assert!(!gate.accept(false));
        assert!(gate.accept(true));
        assert!(gate.accept(false));
    }

    #[test]
    fn a_fragment_is_flagged_as_a_keyframe_only_when_it_opens_on_one() {
        let mut mux = H264Mux::new();
        let nal = vec![vec![0x41, 0x9a]];
        assert!(mux.push(0, true, &nal).is_none());
        let mut flags = Vec::new();
        for index in 1..=FRAGMENT_FRAMES as u64 {
            if let Some((_, keyframe)) = mux.push(index * 40_000, false, &nal) {
                flags.push(keyframe);
            }
        }
        if let Some((_, keyframe)) = mux.flush() {
            flags.push(keyframe);
        }
        assert_eq!(flags, vec![true, false]);
    }

    #[test]
    fn a_real_h264_clip_muxes_into_fmp4_the_browser_can_append() {
        let clip = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/dual-lens/dual-lens-stacked-640x720.h264"
        ))
        .expect("fixture clip");
        let units = crate::media::nal::split_annexb(&clip);
        let sps = units
            .iter()
            .find(|unit| unit.h264_type() == Some(7))
            .map(|unit| unit.data.to_vec())
            .expect("SPS in the fixture");
        let pps = units
            .iter()
            .find(|unit| unit.h264_type() == Some(8))
            .map(|unit| unit.data.to_vec())
            .expect("PPS in the fixture");
        let (init, codec, width, height) = h264_init(&sps, &pps).expect("init segment");
        assert_eq!(
            codec,
            format!("avc1.{:02X}{:02X}{:02X}", sps[1], sps[2], sps[3])
        );
        assert_eq!(&init[4..8], b"ftyp");
        assert!(init.windows(4).any(|window| window == b"avc1"));
        assert!(init.windows(4).any(|window| window == b"avcC"));
        assert!(width >= 16 && height >= 16);

        let mut mux = H264Mux::new();
        let mut stream = init.to_vec();
        let mut frames = 0u64;
        let mut flags: Vec<bool> = Vec::new();
        let mut pending: Vec<Vec<u8>> = Vec::new();
        for unit in &units {
            match unit.h264_type() {
                Some(1) | Some(5) => {
                    pending.push(unit.data.to_vec());
                    let keyframe = pending.iter().any(|nal| h264::is_keyframe(nal));
                    if let Some((fragment, flagged)) = mux.push(frames * 40_000, keyframe, &pending)
                    {
                        stream.extend_from_slice(&fragment);
                        flags.push(flagged);
                    }
                    pending.clear();
                    frames += 1;
                }
                Some(7) | Some(8) => {}
                _ => pending.push(unit.data.to_vec()),
            }
        }
        if let Some((fragment, flagged)) = mux.flush() {
            stream.extend_from_slice(&fragment);
            flags.push(flagged);
        }
        assert!(frames > 10, "{frames} frames");
        assert_eq!(flags, vec![true, true]);

        let mut splitter = Fmp4Splitter::new();
        let pieces = splitter.push(&stream);
        match &pieces[0] {
            Piece::Init(bytes) => assert_eq!(bytes.as_ref(), init.as_ref()),
            Piece::Fragment(_) => panic!("the first piece must be the init segment"),
        }
        assert_eq!(pieces.len() as u64, flags.len() as u64 + 1);
    }
}
