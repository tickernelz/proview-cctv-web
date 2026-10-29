use std::collections::HashSet;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::time::timeout;

use super::client::{describe_ret, require_ok, Connection, DvrError};
use super::crypto::sofia_hash;
use super::frame::{FrameError, Header};

pub const FILE_QUERY: u16 = 1440;
pub const PLAYBACK_CLAIM: u16 = 1424;
pub const PLAYBACK_CONTROL: u16 = 1420;
pub const PLAYBACK_DATA: u16 = 0x058e;
pub const DOWNLOAD_DATA: u16 = 0x0592;
pub const PAGE_FILES: usize = 64;
pub const MAX_FILES: usize = 2048;
pub const REPLY_WAIT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub enum PlaybackError {
    Io(std::io::Error),
    Frame(FrameError),
    NoReply(Duration),
    Malformed(String),
    Rejected { ret: i64, reason: &'static str },
}

impl std::fmt::Display for PlaybackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlaybackError::Io(error) => write!(f, "io: {error}"),
            PlaybackError::Frame(error) => write!(f, "frame: {error:?}"),
            PlaybackError::NoReply(wait) => write!(f, "no reply inside {wait:?}"),
            PlaybackError::Malformed(detail) => write!(f, "unexpected reply: {detail}"),
            PlaybackError::Rejected { ret, reason } => {
                write!(f, "device returned ret {ret}: {reason}")
            }
        }
    }
}

impl std::error::Error for PlaybackError {}

impl From<std::io::Error> for PlaybackError {
    fn from(error: std::io::Error) -> Self {
        PlaybackError::Io(error)
    }
}

impl From<FrameError> for PlaybackError {
    fn from(error: FrameError) -> Self {
        PlaybackError::Frame(error)
    }
}

impl From<DvrError> for PlaybackError {
    fn from(error: DvrError) -> Self {
        match error {
            DvrError::Rejected { ret, reason } => PlaybackError::Rejected { ret, reason },
            other => PlaybackError::Malformed(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlaybackFile {
    pub name: String,
    pub begin: String,
    pub end: String,
    pub bytes: u64,
    pub disk: Option<u64>,
}

fn parse_kilobytes(raw: &str) -> u64 {
    let trimmed = raw.trim();
    let kilobytes = match trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        Some(hex) => u64::from_str_radix(hex, 16).unwrap_or(0),
        None => trimmed.parse().unwrap_or(0),
    };
    kilobytes.saturating_mul(1024)
}

fn text(item: &Value, key: &str) -> String {
    item.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub fn parse_files(reply: &Value) -> Vec<PlaybackFile> {
    let empty = Vec::new();
    let items = reply
        .get("OPFileQuery")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    items
        .iter()
        .filter_map(|item| {
            let name = text(item, "FileName");
            if name.is_empty() {
                return None;
            }
            Some(PlaybackFile {
                name,
                begin: text(item, "BeginTime"),
                end: text(item, "EndTime"),
                bytes: parse_kilobytes(&text(item, "FileLength")),
                disk: item.get("DiskNo").and_then(Value::as_u64),
            })
        })
        .collect()
}

pub fn file_query_body(channel: u8, begin: &str, end: &str, kind: &str) -> Value {
    json!({
        "Name": "OPFileQuery",
        "OPFileQuery": {
            "BeginTime": begin,
            "Channel": channel,
            "DriverTypeMask": "0x0000FFFF",
            "EndTime": end,
            "Event": "*",
            "StreamType": "0x00000000",
            "Type": kind,
        },
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackAction {
    Claim,
    Start,
    Stop,
    DownloadStart,
    DownloadStop,
}

impl PlaybackAction {
    pub fn wire(self) -> &'static str {
        match self {
            PlaybackAction::Claim => "Claim",
            PlaybackAction::Start => "Start",
            PlaybackAction::Stop => "Stop",
            PlaybackAction::DownloadStart => "DownloadStart",
            PlaybackAction::DownloadStop => "DownloadStop",
        }
    }
}

pub fn playback_body(
    session: u32,
    action: PlaybackAction,
    channel: u8,
    name: &str,
    begin: &str,
    end: &str,
) -> Value {
    json!({
        "Name": "OPPlayBack",
        "SessionID": format!("0x{session:08X}"),
        "OPPlayBack": {
            "Action": action.wire(),
            "Parameter": {
                "Channel": channel,
                "FileName": name,
                "PlayMode": "ByName",
                "StreamType": 0,
                "TransMode": "TCP",
                "Value": 0,
            },
            "StartTime": begin,
            "EndTime": end,
        },
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFrame {
    pub id: u16,
    pub channel: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub session: u32,
    pub device_type: Option<String>,
    pub channels: Option<u32>,
}

pub fn absorb(
    seen: &mut HashSet<String>,
    collected: &mut Vec<PlaybackFile>,
    page: Vec<PlaybackFile>,
) -> (usize, String) {
    let last_begin = page
        .last()
        .map(|file| file.begin.clone())
        .unwrap_or_default();
    let mut added = 0;
    for file in page {
        if seen.insert(file.name.clone()) {
            collected.push(file);
            added += 1;
        }
    }
    (added, last_begin)
}

pub struct Session {
    connection: Connection,
    cipher: Option<crate::media::dhav::MediaCipher>,
}

impl Session {
    pub fn open(stream: TcpStream) -> Self {
        Self {
            connection: Connection::from_stream(stream),
            cipher: None,
        }
    }

    pub fn from_connection(
        connection: Connection,
        cipher: crate::media::dhav::MediaCipher,
    ) -> Self {
        Self {
            connection,
            cipher: Some(cipher),
        }
    }

    pub fn session(&self) -> u32 {
        self.connection.session()
    }

    pub fn set_session(&mut self, session: u32) {
        self.connection.set_session(session);
    }

    async fn send_on(
        &mut self,
        channel: u8,
        end: bool,
        id: u16,
        body: &[u8],
    ) -> Result<(), PlaybackError> {
        Ok(self
            .connection
            .send_frame_on(channel, end, id, body)
            .await?)
    }

    pub async fn send_json(&mut self, id: u16, body: &Value) -> Result<(), PlaybackError> {
        let plain = serde_json::to_vec(body)
            .map_err(|error| PlaybackError::Malformed(error.to_string()))?;
        match &self.cipher {
            Some(cipher) => {
                let mut payload = cipher.encrypt_body(&plain).into_bytes();
                payload.push(0);
                self.send_on(0, false, id, &payload).await
            }
            None => {
                let mut payload = plain;
                payload.push(b'\n');
                payload.push(0);
                self.send_on(0, true, id, &payload).await
            }
        }
    }

    pub async fn recv(&mut self) -> Result<(Header, Vec<u8>), PlaybackError> {
        Ok(self.connection.recv().await?)
    }

    async fn send_json_plain(&mut self, id: u16, body: &Value) -> Result<(), PlaybackError> {
        let mut payload = serde_json::to_vec(body)
            .map_err(|error| PlaybackError::Malformed(error.to_string()))?;
        payload.push(b'\n');
        payload.push(0);
        self.send_on(0, false, id, &payload).await
    }

    async fn await_json(&mut self, wait: Duration) -> Result<Value, PlaybackError> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(PlaybackError::NoReply(wait));
            }
            let (_, payload) = match timeout(remaining, self.recv()).await {
                Ok(Ok(frame)) => frame,
                Ok(Err(error)) => return Err(error),
                Err(_) => return Err(PlaybackError::NoReply(wait)),
            };
            let mut end = payload.len();
            while end > 0 && matches!(payload[end - 1], 0 | b'\n' | b'\r') {
                end -= 1;
            }
            if let Ok(value) = serde_json::from_slice::<Value>(&payload[..end]) {
                if value.is_object() {
                    return Ok(value);
                }
            }
            if let Some(cipher) = &self.cipher {
                let opened = cipher.decrypt_body(&payload).and_then(|raw| {
                    let mut end = raw.len();
                    while end > 0 && matches!(raw[end - 1], 0 | b'\n' | b'\r') {
                        end -= 1;
                    }
                    serde_json::from_slice::<Value>(&raw[..end]).ok()
                });
                if let Some(value) = opened.filter(Value::is_object) {
                    return Ok(value);
                }
            }
        }
    }

    pub async fn request(&mut self, id: u16, body: Value) -> Result<Value, PlaybackError> {
        self.send_json(id, &body).await?;
        self.await_json(REPLY_WAIT).await
    }

    pub async fn login(&mut self, user: &str, password: &str) -> Result<Login, PlaybackError> {
        let body = json!({
            "EncryptType": "MD5",
            "LoginType": "DVRIP-Web",
            "PassWord": sofia_hash(password),
            "UserName": user,
        });
        let reply = self.request(1000, body).await?;
        require_ok(&reply)?;
        if let Some(session) = reply
            .get("SessionID")
            .and_then(Value::as_str)
            .and_then(parse_session)
        {
            self.set_session(session);
        }
        Ok(Login {
            session: self.session(),
            device_type: reply
                .get("DeviceType ")
                .or_else(|| reply.get("DeviceType"))
                .and_then(Value::as_str)
                .map(str::to_string),
            channels: reply
                .get("ChannelNum")
                .and_then(Value::as_u64)
                .map(|value| value as u32),
        })
    }

    pub async fn files(
        &mut self,
        channel: u8,
        begin: &str,
        end: &str,
        kind: &str,
    ) -> Result<Vec<PlaybackFile>, PlaybackError> {
        let mut collected: Vec<PlaybackFile> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut cursor = begin.to_string();
        loop {
            let reply = self
                .request(FILE_QUERY, file_query_body(channel, &cursor, end, kind))
                .await?;
            if let Some(ret) = reply.get("Ret").and_then(Value::as_i64) {
                if ret != 100 && ret != 150 {
                    return Err(PlaybackError::Rejected {
                        ret,
                        reason: describe_ret(ret),
                    });
                }
            }
            let page = parse_files(&reply);
            if page.is_empty() {
                break;
            }
            let full = page.len() >= PAGE_FILES;
            let (added, last_begin) = absorb(&mut seen, &mut collected, page);
            if added == 0 || collected.len() >= MAX_FILES {
                break;
            }
            if !full || last_begin.is_empty() || last_begin == cursor {
                break;
            }
            cursor = last_begin;
        }
        Ok(collected)
    }

    pub async fn claim(&mut self, file: &PlaybackFile, channel: u8) -> Result<i64, PlaybackError> {
        let body = playback_body(
            self.session(),
            PlaybackAction::Claim,
            channel,
            &file.name,
            &file.begin,
            &file.end,
        );
        let reply = self.request(PLAYBACK_CLAIM, body).await?;
        Ok(require_ok(&reply)?)
    }

    pub async fn start(&mut self, file: &PlaybackFile, channel: u8) -> Result<(), PlaybackError> {
        let body = playback_body(
            self.session(),
            PlaybackAction::Start,
            channel,
            &file.name,
            &file.begin,
            &file.end,
        );
        self.send_json_plain(PLAYBACK_CONTROL, &body).await
    }

    pub async fn download_start(
        &mut self,
        file: &PlaybackFile,
        channel: u8,
    ) -> Result<(), PlaybackError> {
        let body = playback_body(
            self.session(),
            PlaybackAction::DownloadStart,
            channel,
            &file.name,
            &file.begin,
            &file.end,
        );
        self.send_json_plain(PLAYBACK_CONTROL, &body).await
    }

    pub async fn download_stop(
        &mut self,
        file: &PlaybackFile,
        channel: u8,
    ) -> Result<(), PlaybackError> {
        let body = playback_body(
            self.session(),
            PlaybackAction::DownloadStop,
            channel,
            &file.name,
            &file.begin,
            &file.end,
        );
        self.send_json(PLAYBACK_CONTROL, &body).await
    }

    pub async fn stop(&mut self, file: &PlaybackFile, channel: u8) -> Result<(), PlaybackError> {
        let body = playback_body(
            self.session(),
            PlaybackAction::Stop,
            channel,
            &file.name,
            &file.begin,
            &file.end,
        );
        self.send_json(PLAYBACK_CONTROL, &body).await
    }

    pub async fn frame(&mut self, wait: Duration) -> Result<Option<MediaFrame>, PlaybackError> {
        match timeout(wait, self.recv()).await {
            Ok(Ok((header, payload))) => Ok(Some(MediaFrame {
                id: header.id,
                channel: header.channel,
                payload,
            })),
            Ok(Err(error)) => Err(error),
            Err(_) => Ok(None),
        }
    }
}

pub fn describe(response: i64) -> &'static str {
    describe_ret(response)
}

fn parse_session(raw: &str) -> Option<u32> {
    let trimmed = raw.trim();
    match trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => trimmed.parse().ok(),
    }
}

pub const RECORD_INFO: u8 = 0xf9;
pub const RECORD_PAD: u8 = 0xfa;
pub const RECORD_KEY: u8 = 0xfc;
pub const RECORD_DELTA: u8 = 0xfd;
pub const RECORD_PAD_LEN: usize = 328;
pub const RECORD_INFO_LEN: usize = 16;
pub const RECORD_MAX: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    Info,
    Pad,
    Key,
    Delta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub kind: RecordKind,
    pub stamp: Option<u32>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct MediaReader {
    buffer: Vec<u8>,
    skipped: usize,
}

impl MediaReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    pub fn skipped(&self) -> usize {
        self.skipped
    }

    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }

    fn find_magic(&self) -> Option<usize> {
        self.buffer.windows(4).position(|window| {
            window[0] == 0 && window[1] == 0 && window[2] == 1 && window[3] >= 0xf8
        })
    }

    pub fn next_record(&mut self) -> Option<Record> {
        let start = match self.find_magic() {
            Some(index) => index,
            None => {
                let keep = self.buffer.len().saturating_sub(3);
                self.skipped += keep;
                self.buffer.drain(..keep);
                return None;
            }
        };
        if start > 0 {
            self.skipped += start;
            self.buffer.drain(..start);
        }
        if self.buffer.len() < 8 {
            return None;
        }
        let kind = match self.buffer[3] {
            RECORD_INFO => RecordKind::Info,
            RECORD_PAD => RecordKind::Pad,
            RECORD_KEY => RecordKind::Key,
            RECORD_DELTA => RecordKind::Delta,
            _other => {
                self.skipped += 1;
                self.buffer.drain(..1);
                return self.next_record();
            }
        };
        let (header, body) = match kind {
            RecordKind::Info => (RECORD_INFO_LEN, 0usize),
            RecordKind::Pad => (
                8usize,
                u16::from_le_bytes([self.buffer[6], self.buffer[7]]) as usize,
            ),
            RecordKind::Key => {
                if self.buffer.len() < 16 {
                    return None;
                }
                (
                    16usize,
                    u32::from_le_bytes([
                        self.buffer[12],
                        self.buffer[13],
                        self.buffer[14],
                        self.buffer[15],
                    ]) as usize,
                )
            }
            RecordKind::Delta => (
                8usize,
                u32::from_le_bytes([
                    self.buffer[4],
                    self.buffer[5],
                    self.buffer[6],
                    self.buffer[7],
                ]) as usize,
            ),
        };
        if kind == RecordKind::Pad && body == 0 {
            return self.take(RECORD_PAD_LEN, kind, None);
        }
        if body > RECORD_MAX {
            self.skipped += 1;
            self.buffer.drain(..1);
            return self.next_record();
        }
        let stamp = if kind == RecordKind::Key {
            Some(u32::from_le_bytes([
                self.buffer[8],
                self.buffer[9],
                self.buffer[10],
                self.buffer[11],
            ]))
        } else {
            None
        };
        self.take(header + body, kind, stamp)
    }

    fn take(&mut self, length: usize, kind: RecordKind, stamp: Option<u32>) -> Option<Record> {
        if self.buffer.len() < length {
            return None;
        }
        let mut record = self.buffer.drain(..length).collect::<Vec<u8>>();
        let header = match kind {
            RecordKind::Info => RECORD_INFO_LEN,
            RecordKind::Pad => 8,
            RecordKind::Key => 16,
            RecordKind::Delta => 8,
        };
        let bytes = if kind == RecordKind::Info || kind == RecordKind::Pad {
            Vec::new()
        } else {
            record.split_off(header)
        };
        Some(Record { kind, stamp, bytes })
    }
}

pub fn access_unit(record: &Record) -> Vec<Vec<u8>> {
    crate::media::nal::split_annexb(&record.bytes)
        .into_iter()
        .map(|unit| unit.data.to_vec())
        .collect()
}

pub fn codec_of(nals: &[Vec<u8>]) -> Option<crate::media::Codec> {
    for nal in nals {
        let byte = *nal.first()?;
        let hevc_type = (byte >> 1) & 0x3f;
        if hevc_type == 32 || hevc_type == 33 {
            return Some(crate::media::Codec::H265);
        }
    }
    for nal in nals {
        let byte = *nal.first()?;
        let h264_type = byte & 0x1f;
        if h264_type == 7 || h264_type == 5 {
            return Some(crate::media::Codec::H264);
        }
    }
    None
}

pub fn is_video(record: &Record) -> bool {
    matches!(record.kind, RecordKind::Key | RecordKind::Delta)
}

impl Session {
    pub async fn logout(&mut self) -> Result<(), PlaybackError> {
        let body = json!({
            "Name": "",
            "SessionID": format!("0x{:08X}", self.session()),
        });
        let _ = self.send_json(1001, &body).await;
        self.set_session(0);
        Ok(())
    }
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 } as i64;
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

pub fn shift(timestamp: &str, seconds: u32) -> Option<String> {
    let trimmed = timestamp.trim();
    let (date, clock) = trimmed.split_once(' ')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    let mut clock_parts = clock.split(':');
    let hour: i64 = clock_parts.next()?.parse().ok()?;
    let minute: i64 = clock_parts.next()?.parse().ok()?;
    let second: i64 = clock_parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let total = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second;
    let moved = total + i64::from(seconds);
    let days = moved.div_euclid(86_400);
    let rest = moved.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    Some(format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rest / 3_600,
        (rest % 3_600) / 60,
        rest % 60
    ))
}

pub fn stamp_seconds(raw: u32) -> Option<i64> {
    let second = i64::from(raw & 0x3f);
    let minute = i64::from((raw >> 6) & 0x3f);
    let hour = i64::from((raw >> 12) & 0x1f);
    let day = i64::from((raw >> 17) & 0x1f);
    let month = i64::from((raw >> 22) & 0xf);
    let year = i64::from((raw >> 26) & 0x3f) + 2000;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    Some(
        days_from_civil(year, month as u32, day as u32) * 86_400
            + hour * 3_600
            + minute * 60
            + second,
    )
}

const STANDARD_RATES: [f64; 12] = [
    1.0, 2.0, 3.0, 5.0, 8.0, 10.0, 12.0, 12.5, 15.0, 20.0, 25.0, 30.0,
];
const SETTLE_KEYS: usize = 4;
const SETTLE_SPAN: i64 = 9;
const SNAP_TOLERANCE: f64 = 0.15;

pub fn snap_rate(measured: f64) -> f64 {
    let nearest = STANDARD_RATES.iter().copied().min_by(|left, right| {
        (measured - left)
            .abs()
            .partial_cmp(&(measured - right).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    match nearest {
        Some(candidate) if (measured - candidate).abs() / candidate <= SNAP_TOLERANCE => candidate,
        _ => measured,
    }
}

pub struct FrameRate {
    pub per_second: f64,
    frames: usize,
    keys: usize,
    first: Option<i64>,
    span: i64,
    measured: Option<f64>,
    settled: bool,
}

impl Default for FrameRate {
    fn default() -> Self {
        Self {
            per_second: 25.0,
            frames: 0,
            keys: 0,
            first: None,
            span: 0,
            measured: None,
            settled: false,
        }
    }
}

impl FrameRate {
    pub fn settled(&self) -> bool {
        self.settled
    }

    pub fn keys(&self) -> usize {
        self.keys
    }

    pub fn span_seconds(&self) -> i64 {
        self.span
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    pub fn estimate(&self) -> Option<f64> {
        self.measured
    }

    pub fn observe(&mut self, record: &Record) {
        if !is_video(record) {
            return;
        }
        if self.settled {
            return;
        }
        let seconds = record.stamp.and_then(stamp_seconds);
        match (self.first, seconds) {
            (None, Some(seconds)) => {
                self.first = Some(seconds);
                self.keys = 1;
                self.frames = 1;
            }
            (None, None) => {}
            (Some(first), Some(seconds)) if seconds > first => {
                self.frames += 1;
                self.keys += 1;
                let span = seconds - first;
                if span >= 2 {
                    let measured = (self.frames.saturating_sub(1)) as f64 / span as f64;
                    if (1.0..=60.0).contains(&measured) {
                        self.measured = Some(measured);
                    }
                }
                if self.keys >= SETTLE_KEYS && span >= SETTLE_SPAN {
                    let measured = (self.frames.saturating_sub(1)) as f64 / span as f64;
                    if (1.0..=60.0).contains(&measured) {
                        self.per_second = snap_rate(measured);
                        self.span = span;
                        self.settled = true;
                    }
                }
            }
            _ => {
                self.frames += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn hex(text: &str) -> Vec<u8> {
        let digits: Vec<u8> = text
            .bytes()
            .filter(|byte| byte.is_ascii_hexdigit())
            .collect();
        digits
            .chunks(2)
            .map(|pair| {
                let high = (pair[0] as char).to_digit(16).expect("hex") as u8;
                let low = (pair[1] as char).to_digit(16).expect("hex") as u8;
                (high << 4) | low
            })
            .collect()
    }

    fn key_record(body: &[u8]) -> Vec<u8> {
        let mut out = hex("000001fc530c20440766b36a");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
        out
    }

    fn delta_record(body: &[u8]) -> Vec<u8> {
        let mut out = hex("000001fd");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn a_captured_key_header_carries_its_payload_length() {
        let mut reader = MediaReader::new();
        let body = hex("0000000140010c01ffff0160");
        reader.push(&key_record(&body));
        let first = reader.next_record().expect("record");
        assert_eq!(first.kind, RecordKind::Key);
        assert_eq!(first.stamp, Some(0x6ab36607));
        assert_eq!(first.bytes, body);
    }

    #[test]
    fn a_captured_delta_header_carries_its_payload_length() {
        let mut reader = MediaReader::new();
        let body = hex("000000010201d003102285178d26");
        reader.push(&delta_record(&body));
        let first = reader.next_record().expect("record");
        assert_eq!(first.kind, RecordKind::Delta);
        assert_eq!(first.stamp, None);
        assert_eq!(first.bytes, body);
    }

    #[test]
    fn the_pad_record_is_the_fixed_captured_size() {
        let mut reader = MediaReader::new();
        let mut pad = hex("000001fa0e024001");
        pad.extend([0xd5u8; 320]);
        reader.push(&pad);
        let first = reader.next_record().expect("record");
        assert_eq!(first.kind, RecordKind::Pad);
        assert!(first.bytes.is_empty());
        assert_eq!(reader.buffered(), 0);
    }

    #[test]
    fn the_info_record_is_sixteen_bytes() {
        let mut reader = MediaReader::new();
        reader.push(&hex("000001f90e0008000000000000000000"));
        let first = reader.next_record().expect("record");
        assert_eq!(first.kind, RecordKind::Info);
        assert_eq!(reader.buffered(), 0);
    }

    #[test]
    fn a_record_split_across_two_chunks_waits_for_the_rest() {
        let mut reader = MediaReader::new();
        let body = hex("000000014201010160");
        let whole = key_record(&body);
        let (head, tail) = whole.split_at(10);
        reader.push(head);
        assert!(reader.next_record().is_none());
        reader.push(tail);
        let first = reader.next_record().expect("record");
        assert_eq!(first.bytes, body);
    }

    #[test]
    fn bytes_before_the_magic_are_counted_as_skipped() {
        let mut reader = MediaReader::new();
        let mut junk = vec![0xaa, 0x00, 0x00];
        junk.extend_from_slice(&delta_record(&hex("000000010201")));
        reader.push(&junk);
        let first = reader.next_record().expect("record");
        assert_eq!(first.kind, RecordKind::Delta);
        assert_eq!(reader.skipped(), 3);
    }

    #[test]
    fn the_access_unit_keeps_every_parameter_set_of_a_key_frame() {
        let mut reader = MediaReader::new();
        let body = hex("0000000140010c01ffff0160000000014201010160000000014401c0f183");
        reader.push(&key_record(&body));
        let record = reader.next_record().expect("record");
        let nals = access_unit(&record);
        assert_eq!(nals.len(), 3);
        assert_eq!(nals[0][0] >> 1 & 0x3f, 32);
        assert_eq!(nals[1][0] >> 1 & 0x3f, 33);
        assert_eq!(nals[2][0] >> 1 & 0x3f, 34);
    }

    fn packed(year: i64, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> u32 {
        ((year - 2000) as u32) << 26 | month << 22 | day << 17 | hour << 12 | minute << 6 | second
    }

    fn feed(rate: &mut FrameRate, stamp: Option<u32>, deltas: usize) {
        let mut reader = MediaReader::new();
        reader.push(&key_record(&hex("000000014001")));
        let mut key = reader.next_record().expect("key");
        key.stamp = stamp;
        rate.observe(&key);
        for _ in 0..deltas {
            let mut reader = MediaReader::new();
            reader.push(&delta_record(&hex("000000010201")));
            let delta = reader.next_record().expect("delta");
            rate.observe(&delta);
        }
    }

    #[test]
    fn the_stamp_is_packed_calendar_time_not_seconds() {
        let raw = packed(2026, 10, 25, 22, 24, 7);
        assert_eq!(raw, 1_790_141_959);
        let expected = days_from_civil(2026, 10, 25) * 86_400 + 22 * 3_600 + 24 * 60 + 7;
        assert_eq!(expected, 1_792_967_047);
        assert_eq!(stamp_seconds(raw), Some(expected));
        assert_eq!(stamp_seconds(packed(2026, 0, 25, 22, 24, 7)), None);
        assert_eq!(
            shift("2026-10-24 20:55:00", 123).as_deref(),
            Some("2026-10-24 20:57:03")
        );
    }

    #[test]
    fn the_keyframe_parameter_sets_identify_the_codec() {
        let mut reader = MediaReader::new();
        let body = hex("0000000140010c01ffff0160000000014201010160000000014401c0f183");
        reader.push(&key_record(&body));
        let key = reader.next_record().expect("record");
        assert_eq!(
            codec_of(&access_unit(&key)),
            Some(crate::media::Codec::H265)
        );

        let mut reader = MediaReader::new();
        let body = hex("000000016742001e95a814016e9b808080a0000003002000000f12200000000168ce3c80");
        reader.push(&key_record(&body));
        let key = reader.next_record().expect("record");
        assert_eq!(
            codec_of(&access_unit(&key)),
            Some(crate::media::Codec::H264)
        );
    }

    #[test]
    fn the_frame_rate_ignores_the_second_that_the_packed_stamp_rounds() {
        let mut rate = FrameRate::default();
        feed(&mut rate, Some(packed(2026, 10, 24, 20, 55, 0)), 35);
        feed(&mut rate, Some(packed(2026, 10, 24, 20, 55, 3)), 35);
        assert!(!rate.settled());
        feed(&mut rate, Some(packed(2026, 10, 24, 20, 55, 6)), 35);
        assert!(!rate.settled());
        feed(&mut rate, Some(packed(2026, 10, 24, 20, 55, 9)), 0);
        assert!(rate.settled());
        assert!(
            (rate.per_second - 12.0).abs() < 0.001,
            "{}",
            rate.per_second
        );
    }

    #[test]
    fn the_frame_rate_matches_a_real_gop_of_thirty_six_frames() {
        let mut rate = FrameRate::default();
        for (index, second) in [0u32, 3, 6, 9].into_iter().enumerate() {
            feed(&mut rate, Some(packed(2026, 10, 24, 10, 34, second)), 35);
            if index == 0 {
                assert_eq!(rate.frames(), 36);
            }
        }
        assert!(rate.settled());
        assert!(
            (rate.per_second - 12.0).abs() < 0.001,
            "{}",
            rate.per_second
        );
        assert_eq!(rate.span_seconds(), 9);
    }

    #[test]
    fn the_rate_snapping_prefers_the_nearest_standard_rate() {
        assert_eq!(snap_rate(12.475), 12.5);
        assert_eq!(snap_rate(12.0), 12.0);
        assert_eq!(snap_rate(11.4), 12.0);
        assert_eq!(snap_rate(24.9), 25.0);
        assert_eq!(snap_rate(9.1), 10.0);
        assert_eq!(snap_rate(40.0), 40.0);
    }

    #[test]
    fn an_unsettled_rate_still_reports_its_measurement() {
        let mut rate = FrameRate::default();
        feed(&mut rate, Some(packed(2026, 10, 24, 10, 34, 0)), 35);
        assert_eq!(rate.estimate(), None);
        feed(&mut rate, Some(packed(2026, 10, 24, 10, 34, 3)), 35);
        assert!(!rate.settled());
        let measured = rate.estimate().expect("estimate after two keys");
        assert!((measured - 12.0).abs() < 0.001, "{measured}");
    }

    #[test]
    fn the_frame_rate_survives_a_minute_and_an_hour_boundary() {
        let mut minute = FrameRate::default();
        feed(&mut minute, Some(packed(2026, 10, 24, 20, 55, 54)), 35);
        feed(&mut minute, Some(packed(2026, 10, 24, 20, 55, 57)), 35);
        feed(&mut minute, Some(packed(2026, 10, 24, 20, 56, 0)), 35);
        feed(&mut minute, Some(packed(2026, 10, 24, 20, 56, 3)), 0);
        assert!(
            (minute.per_second - 12.0).abs() < 0.001,
            "{}",
            minute.per_second
        );

        let mut hour = FrameRate::default();
        feed(&mut hour, Some(packed(2026, 10, 24, 20, 59, 54)), 35);
        feed(&mut hour, Some(packed(2026, 10, 24, 20, 59, 57)), 35);
        feed(&mut hour, Some(packed(2026, 10, 24, 21, 0, 0)), 35);
        feed(&mut hour, Some(packed(2026, 10, 24, 21, 0, 3)), 0);
        assert!(
            (hour.per_second - 12.0).abs() < 0.001,
            "{}",
            hour.per_second
        );
    }

    #[test]
    fn shifting_a_device_timestamp_crosses_the_hour_and_the_day() {
        assert_eq!(
            shift("2026-10-24 23:58:00", 300).as_deref(),
            Some("2026-10-25 00:03:00")
        );
        assert_eq!(
            shift("2026-10-24 20:45:00", 125).as_deref(),
            Some("2026-10-24 20:47:05")
        );
    }

    #[test]
    fn shifting_refuses_text_that_is_not_a_device_timestamp() {
        assert_eq!(shift("2026-10-24T20:45:00", 1), None);
        assert_eq!(shift("2026-13-24 20:45:00", 1), None);
        assert_eq!(shift("", 1), None);
    }

    #[test]
    fn a_repeated_page_row_is_not_counted_twice() {
        let file = |name: &str, begin: &str| PlaybackFile {
            name: name.to_string(),
            begin: begin.to_string(),
            end: begin.to_string(),
            bytes: 1,
            disk: None,
        };
        let mut seen = HashSet::new();
        let mut collected = Vec::new();
        let (added, last) = absorb(
            &mut seen,
            &mut collected,
            vec![file("/a", "10:00:00"), file("/b", "10:05:00")],
        );
        assert_eq!(added, 2);
        assert_eq!(last, "10:05:00");
        let (added, _) = absorb(&mut seen, &mut collected, vec![file("/b", "10:05:00")]);
        assert_eq!(added, 0);
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn the_file_length_is_kilobytes_and_matches_a_whole_file_transfer() {
        let reply: Value = serde_json::from_str(
            r#"{"Name":"OPFileQuery","OPFileQuery":[{"BeginTime":"2026-10-24 10:34:00","EndTime":"2026-10-24 10:40:00","FileLength":"0x0000A3E8","FileName":"/idea0/2026-10-24/001/10.34.00-10.40.00[R][@e19][0].h264"}],"Ret":100}"#,
        )
        .expect("json");
        let files = parse_files(&reply);
        assert_eq!(files[0].bytes, 41_960 * 1024);
        assert_eq!(files[0].bytes, 42_967_040);
    }

    #[test]
    fn a_file_list_parses_the_fields_the_device_sends() {
        let reply: Value = serde_json::from_str(
            r#"{"Name":"OPFileQuery","OPFileQuery":[{"BeginTime":"2026-10-24 20:45:00","DiskNo":0,"EndTime":"2026-10-24 20:50:00","FileLength":"0x000082d5","FileName":"/idea0/2026-10-24/001/20.45.00-20.50.00[R][@176d][0].h264","SerialNo":0}],"Ret":100}"#,
        )
        .expect("json");
        let files = parse_files(&reply);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].bytes, 0x82d5 * 1024);
        assert_eq!(files[0].begin, "2026-10-24 20:45:00");
        assert_eq!(files[0].disk, Some(0));
    }

    #[test]
    fn a_query_without_files_is_an_empty_list_not_an_error() {
        let reply: Value =
            serde_json::from_str(r#"{"Name":"OPFileQuery","Ret":109}"#).expect("json");
        assert!(parse_files(&reply).is_empty());
    }

    #[test]
    fn the_query_body_matches_the_wire_shape() {
        let body = file_query_body(1, "2026-10-24 00:00:00", "2026-10-25 00:00:00", "h264");
        let query = body.get("OPFileQuery").expect("block");
        assert_eq!(query.get("Channel").and_then(Value::as_u64), Some(1));
        assert_eq!(query.get("Event").and_then(Value::as_str), Some("*"));
        assert_eq!(query.get("Type").and_then(Value::as_str), Some("h264"));
    }

    #[test]
    fn a_playback_body_carries_the_session_and_the_file() {
        let file = PlaybackFile {
            name: "/idea0/a.h264".to_string(),
            begin: "2026-10-24 20:45:00".to_string(),
            end: "2026-10-24 20:50:00".to_string(),
            bytes: 1,
            disk: None,
        };
        let body = playback_body(
            283,
            PlaybackAction::Start,
            0,
            &file.name,
            &file.begin,
            &file.end,
        );
        assert_eq!(
            body.get("SessionID").and_then(Value::as_str),
            Some("0x0000011B")
        );
        assert_eq!(
            body.pointer("/OPPlayBack/Action").and_then(Value::as_str),
            Some("Start")
        );
        assert_eq!(
            body.pointer("/OPPlayBack/Parameter/FileName")
                .and_then(Value::as_str),
            Some("/idea0/a.h264")
        );
    }

    async fn session_pair() -> (Session, tokio::net::TcpListener) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stream = TcpStream::connect(address).await.unwrap();
        (Session::open(stream), listener)
    }

    fn sample_header() -> [u8; crate::dvrip::HEADER_LEN] {
        Header {
            session: 0x0000_2a2a,
            sequence: 7,
            channel: 3,
            end: true,
            id: 1000,
            length: 5,
        }
        .encode()
    }

    #[tokio::test]
    async fn session_recv_resumes_after_a_half_header_timeout() {
        let (mut session, listener) = session_pair().await;
        let header = sample_header();
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket.write_all(&header[..10]).await.unwrap();
            socket.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(250)).await;
            socket.write_all(&header[10..]).await.unwrap();
            socket.write_all(b"hello").await.unwrap();
            socket.flush().await.unwrap();
        });
        let first = session.frame(Duration::from_millis(80)).await;
        assert_eq!(first.expect("a stall is not an io error"), None);
        let resumed = tokio::time::timeout(Duration::from_secs(2), session.recv())
            .await
            .expect("the resumed read must not hang")
            .expect("frame after the timeout");
        assert_eq!(resumed.0.session, 0x0000_2a2a);
        assert_eq!(resumed.0.id, 1000);
        assert_eq!(resumed.1, b"hello");
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn session_recv_resumes_after_a_half_body_timeout() {
        let (mut session, listener) = session_pair().await;
        let header = sample_header();
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket.write_all(&header).await.unwrap();
            socket.write_all(b"hel").await.unwrap();
            socket.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(250)).await;
            socket.write_all(b"lo").await.unwrap();
            socket.flush().await.unwrap();
        });
        let first = session.frame(Duration::from_millis(80)).await;
        assert_eq!(first.expect("a stall is not an io error"), None);
        let resumed = tokio::time::timeout(Duration::from_secs(2), session.recv())
            .await
            .expect("the resumed read must not hang")
            .expect("frame after the timeout");
        assert_eq!(resumed.0.length, 5);
        assert_eq!(resumed.1, b"hello");
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn a_logged_in_connection_carries_its_session_into_the_file_query() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stream = TcpStream::connect(address).await.unwrap();
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut header = [0u8; crate::dvrip::HEADER_LEN];
            socket.read_exact(&mut header).await.unwrap();
            let login = Header::decode(&header).expect("login header");
            let mut body = vec![0u8; login.length as usize];
            socket.read_exact(&mut body).await.unwrap();
            let reply = b"{\"Ret\":100,\"SessionID\":\"0x44444444\"}\n\0";
            let header = Header {
                session: 0x4444_4444,
                sequence: login.sequence,
                channel: 0,
                end: false,
                id: crate::dvrip::msg::LOGIN_REPLY,
                length: reply.len() as u32,
            }
            .encode();
            socket.write_all(&header).await.unwrap();
            socket.write_all(reply).await.unwrap();
            socket.flush().await.unwrap();
            let mut header = [0u8; crate::dvrip::HEADER_LEN];
            socket.read_exact(&mut header).await.unwrap();
            let asked = Header::decode(&header).expect("query header");
            let mut body = vec![0u8; asked.length as usize];
            socket.read_exact(&mut body).await.unwrap();
            (asked.session, asked.id)
        });
        let mut connection = Connection::from_stream(stream);
        connection.login("admin", "secret").await.expect("login");
        let mut session =
            Session::from_connection(connection, crate::media::dhav::MediaCipher::channel());
        session
            .send_json(
                FILE_QUERY,
                &file_query_body(0, "2026-10-24 00:00:00", "2026-10-25 00:00:00", "h264"),
            )
            .await
            .expect("query");
        let (asked, id) = peer.await.unwrap();
        assert_eq!(asked, 0x4444_4444);
        assert_eq!(id, FILE_QUERY);
        assert_eq!(session.session(), 0x4444_4444);
        drop(session);
    }

    async fn scripted_pair(
        replies: Vec<Vec<u8>>,
    ) -> (
        Session,
        tokio::task::JoinHandle<Vec<(u16, Vec<u8>)>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stream = TcpStream::connect(address).await.unwrap();
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bodies = Vec::new();
            for reply in replies {
                let mut header = [0u8; crate::dvrip::HEADER_LEN];
                if socket.read_exact(&mut header).await.is_err() {
                    break;
                }
                let asked = Header::decode(&header).expect("header");
                let mut body = vec![0u8; asked.length as usize];
                socket.read_exact(&mut body).await.unwrap();
                bodies.push((asked.id, body));
                let response = Header {
                    session: asked.session,
                    sequence: asked.sequence,
                    channel: 0,
                    end: false,
                    id: asked.id + 1,
                    length: reply.len() as u32,
                }
                .encode();
                if socket.write_all(&response).await.is_err() {
                    break;
                }
                let _ = socket.write_all(&reply).await;
                let _ = socket.flush().await;
            }
            bodies
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let session = Session::from_connection(
            Connection::from_stream(stream),
            crate::media::dhav::MediaCipher::channel(),
        );
        (session, peer, tokio::spawn(async {}))
    }

    fn sample_file() -> PlaybackFile {
        PlaybackFile {
            name: "/idea0/2026-10-29/001/07.00.00-07.05.00[R][0].h264".to_string(),
            begin: "2026-10-29 07:00:00".to_string(),
            end: "2026-10-29 07:05:00".to_string(),
            bytes: 0,
            disk: None,
        }
    }

    #[tokio::test]
    async fn a_claim_is_sealed_and_a_start_is_plain() {
        let (mut session, peer, _) =
            scripted_pair(vec![Vec::new(), b"{\"Ret\":100}\n\0".to_vec()]).await;
        session.start(&sample_file(), 0).await.expect("start");
        session.claim(&sample_file(), 0).await.expect("claim");
        let frames = peer.await.unwrap();
        assert_eq!(frames[0].0, PLAYBACK_CONTROL);
        assert_eq!(
            frames[0].1.first(),
            Some(&b'{'),
            "the start must reach the device as plain JSON"
        );
        assert_eq!(frames[1].0, PLAYBACK_CLAIM);
        assert_ne!(
            frames[1].1.first(),
            Some(&b'{'),
            "the claim must not travel as plain JSON"
        );
        let opened = crate::media::dhav::MediaCipher::channel()
            .decrypt_body(&frames[1].1)
            .expect("sealed claim");
        let text = String::from_utf8_lossy(crate::media::dhav::trim_nul(&opened)).into_owned();
        assert!(text.contains("OPPlayBack"), "{text}");
        assert!(text.contains("Claim"), "{text}");
    }

    #[tokio::test]
    async fn a_file_query_that_is_refused_reports_the_ret() {
        let (mut session, _peer, _) = scripted_pair(vec![b"{\"Ret\":117}\n\0".to_vec()]).await;
        let error = session
            .files(0, "2026-10-29 00:00:00", "2026-10-29 23:59:59", "h264")
            .await
            .expect_err("a refusal is not an empty list");
        assert!(error.to_string().contains("ret 117"), "{error}");
        assert!(error.to_string().contains("device error"), "{error}");
    }
}
