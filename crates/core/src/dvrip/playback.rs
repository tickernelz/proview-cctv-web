use std::collections::HashSet;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use super::client::{describe_ret, require_ok, DvrError};
use super::crypto::sofia_hash;
use super::frame::{FrameError, Header};
use super::HEADER_LEN;

pub const FILE_QUERY: u16 = 1440;
pub const PLAYBACK_CLAIM: u16 = 1424;
pub const PLAYBACK_CONTROL: u16 = 1420;
pub const PLAYBACK_DATA: u16 = 0x058e;
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

fn parse_size(raw: &str) -> u64 {
    let trimmed = raw.trim();
    match trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        Some(hex) => u64::from_str_radix(hex, 16).unwrap_or(0),
        None => trimmed.parse().unwrap_or(0),
    }
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
                bytes: parse_size(&text(item, "FileLength")),
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
}

impl PlaybackAction {
    pub fn wire(self) -> &'static str {
        match self {
            PlaybackAction::Claim => "Claim",
            PlaybackAction::Start => "Start",
            PlaybackAction::Stop => "Stop",
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
    stream: TcpStream,
    session: u32,
    sequence: u32,
}

impl Session {
    pub fn open(stream: TcpStream) -> Self {
        stream.set_nodelay(true).ok();
        Self {
            stream,
            session: 0,
            sequence: 0,
        }
    }

    pub fn session(&self) -> u32 {
        self.session
    }

    fn next_sequence(&mut self) -> u32 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }

    async fn send_on(
        &mut self,
        channel: u8,
        end: bool,
        id: u16,
        body: &[u8],
    ) -> Result<(), PlaybackError> {
        let sequence = self.next_sequence();
        let header = Header {
            session: self.session,
            sequence,
            channel,
            end,
            id,
            length: body.len() as u32,
        };
        self.stream.write_all(&header.encode()).await?;
        self.stream.write_all(body).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn send_json(&mut self, id: u16, body: &Value) -> Result<(), PlaybackError> {
        let mut payload = serde_json::to_vec(body)
            .map_err(|error| PlaybackError::Malformed(error.to_string()))?;
        payload.push(b'\n');
        payload.push(0);
        self.send_on(0, true, id, &payload).await
    }

    pub async fn recv(&mut self) -> Result<(Header, Vec<u8>), PlaybackError> {
        let mut header_bytes = [0u8; HEADER_LEN];
        self.stream.read_exact(&mut header_bytes).await?;
        let header = Header::decode(&header_bytes)?;
        let mut body = vec![0u8; header.length as usize];
        if header.length > 0 {
            self.stream.read_exact(&mut body).await?;
        }
        if header.session != 0 {
            self.session = header.session;
        }
        Ok((header, body))
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
            self.session = session;
        }
        Ok(Login {
            session: self.session,
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
            self.session,
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
            self.session,
            PlaybackAction::Start,
            channel,
            &file.name,
            &file.begin,
            &file.end,
        );
        self.send_json(PLAYBACK_CONTROL, &body).await
    }

    pub async fn stop(&mut self, file: &PlaybackFile, channel: u8) -> Result<(), PlaybackError> {
        let body = playback_body(
            self.session,
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

pub fn is_video(record: &Record) -> bool {
    matches!(record.kind, RecordKind::Key | RecordKind::Delta)
}

pub struct FrameRate {
    pub per_second: f64,
    frames: usize,
    first_stamp: Option<u32>,
    last_stamp: Option<u32>,
    settled: bool,
}

impl Default for FrameRate {
    fn default() -> Self {
        Self {
            per_second: 25.0,
            frames: 0,
            first_stamp: None,
            last_stamp: None,
            settled: false,
        }
    }
}

impl FrameRate {
    pub fn observed(record: &Record) -> Option<Self> {
        if record.kind != RecordKind::Key {
            return None;
        }
        let mut rate = Self::default();
        rate.observe(record);
        Some(rate)
    }

    pub fn settled(&self) -> bool {
        self.settled
    }

    pub fn observe(&mut self, record: &Record) {
        if !is_video(record) {
            return;
        }
        if self.settled {
            self.frames += 1;
            return;
        }
        if let Some(stamp) = record.stamp {
            match self.first_stamp {
                None => {
                    self.first_stamp = Some(stamp);
                    self.frames = 1;
                    return;
                }
                Some(first) if stamp > first => {
                    let seconds = f64::from(stamp - first);
                    let per_second = self.frames as f64 / seconds;
                    if (1.0..=60.0).contains(&per_second) {
                        self.per_second = per_second;
                        self.settled = true;
                    }
                    self.last_stamp = Some(stamp);
                    self.frames += 1;
                    return;
                }
                _ => {}
            }
        }
        self.frames += 1;
    }
}

impl Session {
    pub async fn logout(&mut self) -> Result<(), PlaybackError> {
        let body = json!({
            "Name": "",
            "SessionID": format!("0x{:08X}", self.session),
        });
        let _ = self.send_json(1001, &body).await;
        self.session = 0;
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn the_frame_rate_settles_on_the_measured_gop() {
        let mut rate = FrameRate::default();
        let mut reader = MediaReader::new();
        reader.push(&key_record(&hex("000000014001")));
        let mut key = reader.next_record().expect("key");
        key.stamp = Some(100);
        rate.observe(&key);
        for _ in 0..35 {
            let mut reader = MediaReader::new();
            reader.push(&delta_record(&hex("000000010201")));
            let delta = reader.next_record().expect("delta");
            rate.observe(&delta);
        }
        let mut reader = MediaReader::new();
        reader.push(&key_record(&hex("000000014001")));
        let mut next = reader.next_record().expect("key");
        next.stamp = Some(103);
        rate.observe(&next);
        assert!(rate.settled());
        assert!((rate.per_second - 12.0).abs() < 0.01, "{}", rate.per_second);
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
    fn a_file_list_parses_the_fields_the_device_sends() {
        let reply: Value = serde_json::from_str(
            r#"{"Name":"OPFileQuery","OPFileQuery":[{"BeginTime":"2026-10-24 20:45:00","DiskNo":0,"EndTime":"2026-10-24 20:50:00","FileLength":"0x000082d5","FileName":"/idea0/2026-10-24/001/20.45.00-20.50.00[R][@176d][0].h264","SerialNo":0}],"Ret":100}"#,
        )
        .expect("json");
        let files = parse_files(&reply);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].bytes, 0x82d5);
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
}
