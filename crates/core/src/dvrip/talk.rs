use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::crypto::sofia_hash;
use super::frame::{FrameError, Header};
use super::msg;
use crate::media::audio;

pub const MEDIA_MAGIC: [u8; 4] = [0x00, 0x00, 0x01, 0xfa];
pub const CODEC_ALAW: u8 = 0x0e;
pub const RATE_INDEX_8000: u8 = 2;
pub const PACKET_BYTES: usize = audio::FRAME_SAMPLES;

const REPLY_WAIT: Duration = Duration::from_secs(5);
const LOGIN_WAIT: Duration = Duration::from_secs(8);
const READ_LIMIT: usize = 64 * 1024;
const MAX_FRAMES_PER_SECOND: f64 = 30.0;
const BURST_FRAMES: f64 = 5.0;

#[derive(Debug, thiserror::Error)]
pub enum TalkError {
    #[error("device unreachable: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame: {0:?}")]
    Frame(FrameError),
    #[error("device returned ret {ret}: {reason}")]
    Rejected { ret: i64, reason: String },
    #[error("the device sent no reply inside {0:?}")]
    NoReply(Duration),
    #[error("unexpected reply: {0}")]
    BadReply(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TalkStats {
    pub frames: u64,
    pub bytes: u64,
    pub dropped: u64,
    pub inbound_bytes: u64,
    pub inbound_frames: u64,
    pub claim_ms: u64,
    pub seconds: f64,
}

pub fn media_header(packet: usize) -> [u8; 8] {
    let mut header = [0u8; 8];
    header[..4].copy_from_slice(&MEDIA_MAGIC);
    header[4] = CODEC_ALAW;
    header[5] = RATE_INDEX_8000;
    header[6..8].copy_from_slice(&(packet as u16).to_le_bytes());
    header
}

pub fn data_frame(alaw: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(8 + alaw.len());
    frame.extend_from_slice(&media_header(alaw.len()));
    frame.extend_from_slice(alaw);
    frame
}

pub fn talk_body(action: &str, channel: Option<u8>, session: &str) -> Value {
    let mut talk = json!({
        "Action": action,
        "AudioFormat": { "EncodeType": "G711_ALAW" },
    });
    if let Some(channel) = channel {
        talk["Channel"] = json!(channel);
    }
    json!({ "Name": "OPTalk", "SessionID": session, "OPTalk": talk })
}

pub fn frame_delay(sent: u64, elapsed: Duration) -> Option<Duration> {
    let due =
        Duration::from_secs_f64(sent as f64 * PACKET_BYTES as f64 / audio::SAMPLE_RATE as f64);
    due.checked_sub(elapsed)
}

fn trim(payload: &[u8]) -> &[u8] {
    let mut end = payload.len();
    while end > 0 && matches!(payload[end - 1], 0 | b'\n' | b'\r') {
        end -= 1;
    }
    &payload[..end]
}

fn take_frame(buffer: &mut Vec<u8>) -> Option<(Header, Vec<u8>)> {
    if buffer.len() < super::HEADER_LEN {
        return None;
    }
    let header = Header::decode(&buffer[..super::HEADER_LEN]).ok()?;
    let end = super::HEADER_LEN + header.length as usize;
    if buffer.len() < end {
        return None;
    }
    let body = buffer[super::HEADER_LEN..end].to_vec();
    buffer.drain(..end);
    Some((header, body))
}

pub struct TalkLink {
    stream: TcpStream,
    session: u32,
    sequence: u32,
    pending: Vec<u8>,
}

impl TalkLink {
    pub async fn connect(address: &str) -> Result<Self, TalkError> {
        let stream = TcpStream::connect(address).await?;
        stream.set_nodelay(true).ok();
        Ok(Self::from_stream(stream))
    }

    pub fn from_stream(stream: TcpStream) -> Self {
        stream.set_nodelay(true).ok();
        Self {
            stream,
            session: 0,
            sequence: 0,
            pending: Vec::new(),
        }
    }

    pub fn session(&self) -> u32 {
        self.session
    }

    pub fn session_id(&self) -> String {
        format!("0x{:08X}", self.session)
    }

    async fn send_frame(&mut self, id: u16, body: &[u8]) -> Result<(), TalkError> {
        self.sequence = self.sequence.wrapping_add(1);
        let header = Header {
            session: self.session,
            sequence: self.sequence,
            channel: 0,
            end: false,
            id,
            length: body.len() as u32,
        };
        self.stream.write_all(&header.encode()).await?;
        self.stream.write_all(body).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn send(&mut self, id: u16, body: &[u8]) -> Result<(), TalkError> {
        self.send_frame(id, body).await
    }

    pub async fn send_json(&mut self, id: u16, body: &Value) -> Result<(), TalkError> {
        let mut payload =
            serde_json::to_vec(body).map_err(|error| TalkError::BadReply(error.to_string()))?;
        payload.push(b'\n');
        payload.push(0);
        self.send_frame(id, &payload).await
    }

    async fn fill(&mut self, wait: Duration) -> Result<(), TalkError> {
        let mut chunk = [0u8; 4096];
        let read = timeout(wait, self.stream.read(&mut chunk))
            .await
            .map_err(|_| TalkError::NoReply(wait))??;
        if read == 0 {
            return Err(TalkError::NoReply(wait));
        }
        self.pending.extend_from_slice(&chunk[..read]);
        Ok(())
    }

    fn poll_replies(&mut self) -> Vec<Value> {
        let mut replies = Vec::new();
        while let Some((header, body)) = take_frame(&mut self.pending) {
            if header.session != 0 {
                self.session = header.session;
            }
            if let Ok(value) = serde_json::from_slice::<Value>(trim(&body)) {
                if value.is_object() {
                    replies.push(value);
                }
            }
        }
        replies
    }

    pub async fn read_json(&mut self, wait: Duration) -> Result<Value, TalkError> {
        let deadline = Instant::now() + wait;
        loop {
            if let Some(reply) = self.poll_replies().into_iter().next() {
                return Ok(reply);
            }
            let left = deadline
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())
                .ok_or(TalkError::NoReply(wait))?;
            match self.fill(left.min(Duration::from_millis(500))).await {
                Ok(()) => {}
                Err(TalkError::NoReply(_)) => {}
                Err(other) => return Err(other),
            }
        }
    }

    pub async fn login(&mut self, user: &str, password: &str) -> Result<(), TalkError> {
        let body = json!({
            "EncryptType": "MD5",
            "LoginType": "DVRIP-Web",
            "PassWord": sofia_hash(password),
            "UserName": user,
        });
        self.send_json(msg::LOGIN, &body).await?;
        let reply = self.read_json(LOGIN_WAIT).await?;
        let ret = reply
            .get("Ret")
            .and_then(Value::as_i64)
            .ok_or_else(|| TalkError::BadReply("login reply has no Ret".to_string()))?;
        if ret != 100 && ret != 150 {
            return Err(TalkError::Rejected {
                ret,
                reason: super::client::describe_ret(ret).to_string(),
            });
        }
        if let Some(raw) = reply.get("SessionID").and_then(Value::as_str) {
            let trimmed = raw.trim();
            let parsed = trimmed
                .strip_prefix("0x")
                .or_else(|| trimmed.strip_prefix("0X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| trimmed.parse::<u32>().ok());
            if let Some(session) = parsed {
                self.session = session;
            }
        }
        Ok(())
    }

    pub async fn close(mut self) {
        let _ = self.stream.shutdown().await;
    }

    fn into_parts(self) -> (OwnedReadHalf, OwnedWriteHalf, u32, u32, Vec<u8>) {
        let (reader, writer) = self.stream.into_split();
        (reader, writer, self.session, self.sequence, self.pending)
    }
}

struct Credit {
    tokens: f64,
    at: Instant,
}

impl Credit {
    fn new() -> Self {
        Self {
            tokens: BURST_FRAMES,
            at: Instant::now(),
        }
    }

    fn take(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + MAX_FRAMES_PER_SECOND * elapsed).min(BURST_FRAMES);
        self.at = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

pub struct TalkSession {
    writer: OwnedWriteHalf,
    session: u32,
    sequence: u32,
    channel: Option<u8>,
    frames: u64,
    bytes: u64,
    dropped: u64,
    credit: Credit,
    claim: Duration,
    opened: Instant,
    inbound_bytes: Arc<AtomicU64>,
    inbound_frames: Arc<AtomicU64>,
    reader: JoinHandle<()>,
}

async fn send_frame(
    writer: &mut OwnedWriteHalf,
    session: u32,
    sequence: u32,
    id: u16,
    body: &[u8],
) -> Result<u32, TalkError> {
    let sequence = sequence.wrapping_add(1);
    let header = Header {
        session,
        sequence,
        channel: 0,
        end: false,
        id,
        length: body.len() as u32,
    };
    writer.write_all(&header.encode()).await?;
    writer.write_all(body).await?;
    writer.flush().await?;
    Ok(sequence)
}

async fn send_json(
    writer: &mut OwnedWriteHalf,
    session: u32,
    sequence: u32,
    id: u16,
    body: &Value,
) -> Result<u32, TalkError> {
    let mut payload =
        serde_json::to_vec(body).map_err(|error| TalkError::BadReply(error.to_string()))?;
    payload.push(b'\n');
    payload.push(0);
    send_frame(writer, session, sequence, id, &payload).await
}

async fn read_loop(
    mut reader: OwnedReadHalf,
    pending: Vec<u8>,
    bytes: Arc<AtomicU64>,
    frames: Arc<AtomicU64>,
    audio: Option<Sender<(u16, Vec<u8>)>>,
) {
    {
        let mut buffer = pending;
        let mut chunk = [0u8; READ_LIMIT];
        loop {
            while let Some((header, body)) = take_frame(&mut buffer) {
                frames.fetch_add(1, Ordering::Relaxed);
                if let Some(sink) = audio.as_ref() {
                    let _ = sink.try_send((header.id, body));
                }
            }
            match reader.read(&mut chunk).await {
                Ok(0) => break,
                Ok(read) => {
                    bytes.fetch_add(read as u64, Ordering::Relaxed);
                    buffer.extend_from_slice(&chunk[..read]);
                }
                Err(_) => break,
            }
        }
    }
}

impl TalkSession {
    pub async fn open(link: TalkLink, channel: Option<u8>) -> Result<Self, TalkError> {
        Self::open_with_audio(link, channel, None).await
    }

    pub async fn open_with_audio(
        mut link: TalkLink,
        channel: Option<u8>,
        audio: Option<Sender<(u16, Vec<u8>)>>,
    ) -> Result<Self, TalkError> {
        let session = link.session_id();
        let claim = talk_body("Claim", channel, &session);
        let started = Instant::now();
        link.send_json(msg::TALK_CLAIM, &claim).await?;
        let reply = link.read_json(REPLY_WAIT).await?;
        let ret = reply
            .get("Ret")
            .and_then(Value::as_i64)
            .ok_or_else(|| TalkError::BadReply("claim reply has no Ret".to_string()))?;
        if ret != 100 {
            return Err(TalkError::Rejected {
                ret,
                reason: super::client::describe_ret(ret).to_string(),
            });
        }
        let claim_latency = started.elapsed();
        link.send_json(msg::TALK, &talk_body("Start", channel, &session))
            .await?;

        let (reader, writer, session, sequence, pending) = link.into_parts();
        let inbound_bytes = Arc::new(AtomicU64::new(0));
        let inbound_frames = Arc::new(AtomicU64::new(0));
        let handle = tokio::spawn(read_loop(
            reader,
            pending,
            inbound_bytes.clone(),
            inbound_frames.clone(),
            audio,
        ));
        Ok(Self {
            writer,
            session,
            sequence,
            channel,
            frames: 0,
            bytes: 0,
            dropped: 0,
            credit: Credit::new(),
            claim: claim_latency,
            opened: Instant::now(),
            inbound_bytes,
            inbound_frames,
            reader: handle,
        })
    }

    pub fn stats(&self) -> TalkStats {
        TalkStats {
            frames: self.frames,
            bytes: self.bytes,
            dropped: self.dropped,
            inbound_bytes: self.inbound_bytes.load(Ordering::Relaxed),
            inbound_frames: self.inbound_frames.load(Ordering::Relaxed),
            claim_ms: self.claim.as_millis() as u64,
            seconds: self.opened.elapsed().as_secs_f64(),
        }
    }

    pub fn channel(&self) -> Option<u8> {
        self.channel
    }

    pub async fn send_alaw(&mut self, alaw: &[u8]) -> Result<usize, TalkError> {
        if alaw.is_empty() {
            return Ok(0);
        }
        let mut sent = 0;
        for chunk in alaw.chunks(PACKET_BYTES) {
            if !self.credit.take() {
                self.dropped += 1;
                continue;
            }
            let mut payload = chunk.to_vec();
            if payload.len() < PACKET_BYTES {
                audio::pad_to_frame(&mut payload, PACKET_BYTES);
            }
            self.sequence = send_frame(
                &mut self.writer,
                self.session,
                self.sequence,
                msg::TALK_DATA,
                &data_frame(&payload),
            )
            .await?;
            self.frames += 1;
            self.bytes += payload.len() as u64;
            sent += 1;
        }
        Ok(sent)
    }

    pub async fn stop(&mut self) -> Result<(), TalkError> {
        let session = format!("0x{:08X}", self.session);
        self.sequence = send_json(
            &mut self.writer,
            self.session,
            self.sequence,
            msg::TALK,
            &talk_body("Stop", self.channel, &session),
        )
        .await?;
        Ok(())
    }

    pub async fn logout(mut self) -> Result<(), TalkError> {
        let session = format!("0x{:08X}", self.session);
        let sent = send_json(
            &mut self.writer,
            self.session,
            self.sequence,
            msg::LOGOUT,
            &json!({ "Name": "", "SessionID": session }),
        )
        .await;
        if let Ok(sequence) = sent {
            self.sequence = sequence;
        }
        let _ = self.writer.shutdown().await;
        let _ = timeout(Duration::from_secs(2), self.reader).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn decode_frame(header: &[u8], body: &[u8]) -> (Header, Vec<u8>) {
        let parsed = Header::decode(header).expect("header");
        assert_eq!(parsed.length as usize, body.len());
        (parsed, body.to_vec())
    }

    async fn read_frame(socket: &mut TcpStream) -> (Header, Vec<u8>) {
        let mut header = [0u8; super::super::HEADER_LEN];
        socket.read_exact(&mut header).await.expect("header");
        let parsed = Header::decode(&header).expect("decodes");
        let mut body = vec![0u8; parsed.length as usize];
        socket.read_exact(&mut body).await.expect("body");
        decode_frame(&header, &body)
    }

    fn json_of(body: &[u8]) -> Value {
        serde_json::from_slice(trim(body)).expect("json")
    }

    async fn write_json_frame(
        socket: &mut TcpStream,
        id: u16,
        session: u32,
        sequence: u32,
        body: Value,
    ) {
        let reply = serde_json::to_vec(&body).unwrap();
        let header = Header {
            session,
            sequence,
            channel: 0,
            end: false,
            id,
            length: reply.len() as u32,
        };
        socket.write_all(&header.encode()).await.unwrap();
        socket.write_all(&reply).await.unwrap();
        socket.flush().await.unwrap();
    }

    #[test]
    fn the_media_header_carries_the_captured_shape() {
        let header = media_header(320);
        assert_eq!(&header[..4], &[0x00, 0x00, 0x01, 0xfa]);
        assert_eq!(header[4], 0x0e);
        assert_eq!(header[5], 0x02);
        assert_eq!(u16::from_le_bytes([header[6], header[7]]), 320);
    }

    #[test]
    fn a_data_frame_is_the_header_followed_by_the_payload() {
        let payload = vec![0xd5u8; 320];
        let frame = data_frame(&payload);
        assert_eq!(frame.len(), 328);
        assert_eq!(&frame[8..], &payload[..]);
    }

    #[test]
    fn the_claim_body_names_optalk_and_the_codec() {
        let body = talk_body("Claim", None, "0x12345678");
        assert_eq!(body["Name"], "OPTalk");
        assert_eq!(body["SessionID"], "0x12345678");
        assert_eq!(body["OPTalk"]["Action"], "Claim");
        assert_eq!(body["OPTalk"]["AudioFormat"]["EncodeType"], "G711_ALAW");
        assert!(body["OPTalk"].get("Channel").is_none());
        let with_channel = talk_body("Start", Some(1), "0x1");
        assert_eq!(with_channel["OPTalk"]["Channel"], 1);
    }

    #[test]
    fn pacing_returns_none_once_the_stream_is_late() {
        assert_eq!(
            frame_delay(0, Duration::from_millis(0)),
            Some(Duration::from_millis(0))
        );
        let late = frame_delay(1, Duration::from_millis(80));
        assert!(late.is_none());
        let ahead = frame_delay(2, Duration::from_millis(0)).expect("due");
        assert!((ahead.as_secs_f64() - 0.080).abs() < 0.001);
    }

    #[test]
    fn the_credit_refills_but_never_past_the_burst() {
        let mut credit = Credit::new();
        for _ in 0..BURST_FRAMES as usize {
            assert!(credit.take());
        }
        assert!(!credit.take());
        std::thread::sleep(Duration::from_millis(120));
        credit.take();
        let mut idle = Credit::new();
        idle.at = Instant::now() - Duration::from_secs(60);
        let mut allowed = 0;
        for _ in 0..20 {
            if idle.take() {
                allowed += 1;
            }
        }
        assert_eq!(allowed, BURST_FRAMES as usize);
    }

    #[tokio::test]
    async fn login_claim_start_and_data_hit_the_wire_in_order() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let (login, login_body) = read_frame(&mut socket).await;
            assert_eq!(login.id, msg::LOGIN);
            let parsed = json_of(&login_body);
            assert_eq!(parsed["EncryptType"], "MD5");
            assert_eq!(parsed["PassWord"], sofia_hash("hunter"));
            write_json_frame(
                &mut socket,
                msg::LOGIN,
                0xdeadbeef,
                1,
                serde_json::json!({ "Ret": 100, "SessionID": "0xdeadbeef" }),
            )
            .await;

            let (claim, claim_body) = read_frame(&mut socket).await;
            assert_eq!(claim.id, msg::TALK_CLAIM);
            assert_eq!(claim.session, 0xdeadbeef);
            let parsed = json_of(&claim_body);
            assert_eq!(parsed["OPTalk"]["Action"], "Claim");
            assert_eq!(parsed["SessionID"], "0xDEADBEEF");
            write_json_frame(
                &mut socket,
                msg::TALK_CLAIM,
                0xdeadbeef,
                2,
                serde_json::json!({ "Ret": 100, "Name": "OPTalk" }),
            )
            .await;

            let (start, start_body) = read_frame(&mut socket).await;
            assert_eq!(start.id, msg::TALK);
            assert_eq!(json_of(&start_body)["OPTalk"]["Action"], "Start");

            let (data, data_body) = read_frame(&mut socket).await;
            assert_eq!(data.id, msg::TALK_DATA);
            assert_eq!(data_body.len(), 8 + PACKET_BYTES);
            assert_eq!(&data_body[..4], &MEDIA_MAGIC);
            assert_eq!(data_body[4], CODEC_ALAW);
            assert_eq!(data_body[5], RATE_INDEX_8000);
            assert_eq!(
                u16::from_le_bytes([data_body[6], data_body[7]]),
                PACKET_BYTES as u16
            );
            assert!(data_body[8..].iter().all(|byte| *byte == audio::SILENCE));

            let (stop, stop_body) = read_frame(&mut socket).await;
            assert_eq!(stop.id, msg::TALK);
            assert_eq!(json_of(&stop_body)["OPTalk"]["Action"], "Stop");
        });

        let mut link = TalkLink::connect(&address.to_string()).await.expect("link");
        link.login("admin", "hunter").await.expect("login");
        assert_eq!(link.session(), 0xdeadbeef);
        let mut session = TalkSession::open(link, None).await.expect("open");
        let payload = vec![audio::SILENCE; PACKET_BYTES];
        assert_eq!(session.send_alaw(&payload).await.expect("send"), 1);
        let stats = session.stats();
        assert_eq!(stats.frames, 1);
        assert_eq!(stats.bytes, PACKET_BYTES as u64);
        session.stop().await.expect("stop");
        server.await.expect("server");
    }

    #[tokio::test]
    async fn audio_the_camera_sends_back_is_read_and_counted() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let _ = read_frame(&mut socket).await;
            write_json_frame(
                &mut socket,
                msg::LOGIN,
                1,
                1,
                serde_json::json!({ "Ret": 100, "SessionID": "0x1" }),
            )
            .await;
            let _ = read_frame(&mut socket).await;
            write_json_frame(
                &mut socket,
                msg::TALK_CLAIM,
                1,
                2,
                serde_json::json!({ "Ret": 100 }),
            )
            .await;
            let _ = read_frame(&mut socket).await;
            for sequence in 0..3u32 {
                let body = data_frame(&vec![0x33u8; PACKET_BYTES]);
                let header = Header {
                    session: 1,
                    sequence,
                    channel: 0,
                    end: false,
                    id: msg::TALK_DATA,
                    length: body.len() as u32,
                };
                socket.write_all(&header.encode()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            }
            socket.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
        });

        let mut link = TalkLink::connect(&address.to_string()).await.expect("link");
        link.login("admin", "hunter").await.expect("login");
        let session = TalkSession::open(link, None).await.expect("open");
        let mut observed = 0;
        for _ in 0..40 {
            let stats = session.stats();
            if stats.inbound_frames >= 3 {
                observed = stats.inbound_frames;
                assert!(stats.inbound_bytes >= 3 * 328);
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(observed >= 3, "inbound frames {observed}");
        server.await.expect("server");
    }

    #[tokio::test]
    async fn a_claim_the_device_refuses_is_reported_with_its_ret() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let _ = read_frame(&mut socket).await;
            write_json_frame(
                &mut socket,
                msg::LOGIN,
                1,
                1,
                serde_json::json!({ "Ret": 100, "SessionID": "0x1" }),
            )
            .await;
            let _ = read_frame(&mut socket).await;
            write_json_frame(
                &mut socket,
                msg::TALK_CLAIM,
                1,
                2,
                serde_json::json!({ "Ret": 103 }),
            )
            .await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        });

        let mut link = TalkLink::connect(&address.to_string()).await.expect("link");
        link.login("admin", "hunter").await.expect("login");
        let error = match TalkSession::open(link, Some(1)).await {
            Ok(_) => panic!("the claim was accepted"),
            Err(error) => error,
        };
        match error {
            TalkError::Rejected { ret, ref reason } => {
                assert_eq!(ret, 103);
                assert_eq!(reason, "request not permitted");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_flood_is_capped_near_realtime_and_counted() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let _ = read_frame(&mut socket).await;
            write_json_frame(
                &mut socket,
                msg::LOGIN,
                1,
                1,
                serde_json::json!({ "Ret": 100, "SessionID": "0x1" }),
            )
            .await;
            let _ = read_frame(&mut socket).await;
            write_json_frame(
                &mut socket,
                msg::TALK_CLAIM,
                1,
                2,
                serde_json::json!({ "Ret": 100 }),
            )
            .await;
            let _ = read_frame(&mut socket).await;
            tokio::time::sleep(Duration::from_millis(2000)).await;
        });

        let mut link = TalkLink::connect(&address.to_string()).await.expect("link");
        link.login("admin", "hunter").await.expect("login");
        let mut session = TalkSession::open(link, None).await.expect("open");
        tokio::time::sleep(Duration::from_millis(1200)).await;
        let flood = vec![audio::SILENCE; PACKET_BYTES * 20];
        let sent = session.send_alaw(&flood).await.expect("send");
        let stats = session.stats();
        assert_eq!(sent, BURST_FRAMES as usize);
        assert_eq!(stats.frames, sent as u64);
        assert_eq!(stats.dropped, 20 - sent as u64);
        server.await.expect("server");
    }

    #[tokio::test]
    async fn short_payloads_are_padded_to_a_whole_packet() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let _ = read_frame(&mut socket).await;
            write_json_frame(
                &mut socket,
                msg::LOGIN,
                1,
                1,
                serde_json::json!({ "Ret": 100, "SessionID": "0x1" }),
            )
            .await;
            let _ = read_frame(&mut socket).await;
            write_json_frame(
                &mut socket,
                msg::TALK_CLAIM,
                1,
                2,
                serde_json::json!({ "Ret": 100 }),
            )
            .await;
            let _ = read_frame(&mut socket).await;
            let (data, body) = read_frame(&mut socket).await;
            assert_eq!(data.id, msg::TALK_DATA);
            assert_eq!(body.len(), 8 + PACKET_BYTES);
            assert_eq!(
                &body[8 + 100..],
                &vec![audio::SILENCE; PACKET_BYTES - 100][..]
            );
        });

        let mut link = TalkLink::connect(&address.to_string()).await.expect("link");
        link.login("admin", "hunter").await.expect("login");
        let mut session = TalkSession::open(link, None).await.expect("open");
        let sent = vec![0x11u8; 100];
        assert_eq!(session.send_alaw(&sent).await.expect("send"), 1);
        server.await.expect("server");
    }
}
