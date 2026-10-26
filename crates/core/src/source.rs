use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

use crate::live::{Fragment, LiveHub, LiveMode, LivePacket, StreamKey, StreamMeta};
use crate::media::{hevc, rtp};
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
    PassthroughNeedsHevc(String),
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
                "ffmpeg is not installed; set PROVIEW_FFMPEG or install ffmpeg for HEVC cameras"
            ),
            SourceError::PassthroughNeedsHevc(codec) => write!(
                f,
                "passthrough needs an H.265 camera, this one announces {codec}"
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

pub fn ffmpeg_command(codec: &str) -> Command {
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
    ]);
    if codec.eq_ignore_ascii_case("H264") {
        command.args(["-f", "h264", "-i", "pipe:0", "-c:v", "copy"]);
    } else {
        command.args([
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
    }
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

async fn drain_stderr(mut child: Child) -> Option<String> {
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
    if let (Some(vps), Some(sps), Some(pps)) = (&video.vps, &video.sps, &video.pps) {
        if let Ok(config) = hevc_config(vps, sps, pps) {
            meta.width = Some(config.width);
            meta.height = Some(config.height);
            meta.codec_string = Some(config.codec.clone());
        }
    }
    hub.set_meta(key, meta.clone());

    match key.mode {
        LiveMode::Passthrough => {
            pump_passthrough(hub, key, session, video, audio_channel, meta, stop).await
        }
        LiveMode::Transcode => pump_transcode(hub, key, session, video, audio_channel, stop).await,
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
    let mut child = ffmpeg_command(&video.codec)
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

    let parameter_sets: Vec<u8> = [video.vps.as_ref(), video.sps.as_ref(), video.pps.as_ref()]
        .into_iter()
        .flatten()
        .fold(Vec::new(), |mut acc, parameter| {
            hevc::append_annexb(parameter, &mut acc);
            acc
        });
    if !parameter_sets.is_empty() {
        stdin
            .write_all(&parameter_sets)
            .await
            .map_err(SourceError::Io)?;
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
            hub.note_audio(key);
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

struct HevcConfig {
    hvcc: Vec<u8>,
    codec: String,
    width: u32,
    height: u32,
}

fn hevc_config(vps: &[u8], sps: &[u8], pps: &[u8]) -> Result<HevcConfig, String> {
    let parsed = hevc::parse_sps(sps)?;
    Ok(HevcConfig {
        hvcc: hevc::build_hvcc(&parsed, vps, sps, pps),
        codec: hevc::codec_string(&parsed.profile),
        width: parsed.width,
        height: parsed.height,
    })
}

struct ParameterSets {
    vps: Option<Vec<u8>>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
}

fn parameter_sets(track: &Track) -> ParameterSets {
    ParameterSets {
        vps: track.vps.clone(),
        sps: track.sps.clone(),
        pps: track.pps.clone(),
    }
}

fn ensure_parameter_sets(nals: &mut Vec<Vec<u8>>, sets: &ParameterSets) {
    let has = |kind: u8| nals.iter().any(|nal| hevc::nal_type(nal) == Some(kind));
    let mut prefix = Vec::new();
    if !has(32) {
        if let Some(vps) = &sets.vps {
            prefix.push(vps.clone());
        }
    }
    if !has(33) {
        if let Some(sps) = &sets.sps {
            prefix.push(sps.clone());
        }
    }
    if !has(34) {
        if let Some(pps) = &sets.pps {
            prefix.push(pps.clone());
        }
    }
    if prefix.is_empty() {
        return;
    }
    prefix.extend(std::mem::take(nals));
    *nals = prefix;
}

#[allow(clippy::too_many_arguments)]
async fn pump_passthrough(
    hub: &Arc<LiveHub>,
    key: &StreamKey,
    mut session: RtspSession,
    video: Track,
    audio_channel: Option<u8>,
    mut meta: StreamMeta,
    stop: &Arc<AtomicBool>,
) -> Result<(), SourceError> {
    if !video.codec.eq_ignore_ascii_case("H265") {
        return Err(SourceError::PassthroughNeedsHevc(video.codec.clone()));
    }
    let sets = parameter_sets(&video);
    let mut config = match (&sets.vps, &sets.sps, &sets.pps) {
        (Some(vps), Some(sps), Some(pps)) => hevc_config(vps, sps, pps).ok(),
        _ => None,
    };
    let _sender = hub.publisher(key.clone());
    if let Some(config) = config.as_ref() {
        hub.publish_packet(key, LivePacket::config(Fragment::from(config.hvcc.clone())));
    }

    let mut depacketizer = hevc::Depacketizer::new();
    let mut assembler = hevc::Assembler::new();
    let mut started = false;
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
            hub.note_audio(key);
            continue;
        }
        let Some(parsed) = rtp::parse(&packet) else {
            continue;
        };
        if parsed.payload_type != video.payload_type {
            continue;
        }
        let nals = depacketizer.push(&parsed);
        let Some(unit) = assembler.packet(parsed.sequence, parsed.marker, &nals, parsed.timestamp)
        else {
            continue;
        };
        if config.is_none() {
            if let (Some(vps), Some(sps), Some(pps)) = (
                unit.nals.iter().find(|nal| hevc::nal_type(nal) == Some(32)),
                unit.nals.iter().find(|nal| hevc::nal_type(nal) == Some(33)),
                unit.nals.iter().find(|nal| hevc::nal_type(nal) == Some(34)),
            ) {
                if let Ok(built) = hevc_config(vps, sps, pps) {
                    meta.width = Some(built.width);
                    meta.height = Some(built.height);
                    meta.codec_string = Some(built.codec.clone());
                    hub.set_meta(key, meta.clone());
                    hub.publish_packet(key, LivePacket::config(Fragment::from(built.hvcc.clone())));
                    config = Some(built);
                }
            }
        }
        if !unit.keyframe && !started {
            hub.note_drop(key);
            continue;
        }
        if unit.keyframe {
            started = true;
        }
        let mut nals = unit.nals;
        if unit.keyframe {
            ensure_parameter_sets(&mut nals, &sets);
        }
        units += 1;
        hub.publish_packet(
            key,
            LivePacket::access_unit(
                Fragment::from(hevc::avcc(&nals)),
                unit.keyframe,
                unit.pts_us,
            ),
        );
    };

    tracing::info!(
        units,
        dropped = depacketizer.dropped(),
        "passthrough source finished"
    );
    hub.publisher_done(key);
    let _ = tokio::time::timeout(Duration::from_secs(2), session.teardown()).await;
    result
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
    fn ffmpeg_args_transcode_hevc_and_copy_h264() {
        let hevc = ffmpeg_command("H265");
        let args: Vec<String> = hevc
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert!(args.windows(2).any(|pair| pair == ["-f", "hevc"]));
        assert!(args.contains(&"libx264".to_string()));
        assert!(args.contains(&"pipe:1".to_string()));

        let h264 = ffmpeg_command("H264");
        let args: Vec<String> = h264
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert!(args.windows(2).any(|pair| pair == ["-f", "h264"]));
        assert!(args.windows(2).any(|pair| pair == ["-c:v", "copy"]));
        assert!(!args.contains(&"libx264".to_string()));
    }

    #[test]
    fn parameter_sets_are_prepended_when_the_stream_omits_them() {
        let sets = ParameterSets {
            vps: Some(vec![0x40, 0x01, 0x0c]),
            sps: Some(vec![0x42, 0x01, 0x01]),
            pps: Some(vec![0x44, 0x01, 0xc0]),
        };
        let mut nals = vec![vec![0x26, 0x01, 0xaf]];
        ensure_parameter_sets(&mut nals, &sets);
        assert_eq!(nals.len(), 4);
        assert_eq!(hevc::nal_type(&nals[0]), Some(32));
        assert_eq!(hevc::nal_type(&nals[1]), Some(33));
        assert_eq!(hevc::nal_type(&nals[2]), Some(34));
        assert_eq!(hevc::nal_type(&nals[3]), Some(19));
    }

    #[test]
    fn audio_clock_is_known_for_the_g711_codecs() {
        assert_eq!(audio_clock("PCMA"), Some(8000));
        assert_eq!(audio_clock("PCMU"), Some(8000));
        assert_eq!(audio_clock("AAC"), None);
    }
}
