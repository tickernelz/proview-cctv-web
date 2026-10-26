use std::time::Duration;

use base64::Engine as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::dvrip::crypto::md5_hex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdpInfo {
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub framerate: Option<u32>,
    pub control: Option<String>,
}

#[derive(Debug)]
pub enum RtspError {
    Io(std::io::Error),
    Unauthorized,
    Status(String),
    Parse(String),
}

impl std::fmt::Display for RtspError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RtspError::Io(e) => write!(f, "io: {e}"),
            RtspError::Unauthorized => write!(f, "device rejected the credentials"),
            RtspError::Status(s) => write!(f, "unexpected status: {s}"),
            RtspError::Parse(s) => write!(f, "could not parse: {s}"),
        }
    }
}

impl std::error::Error for RtspError {}

pub struct Digest {
    pub realm: String,
    pub nonce: String,
}

pub fn digest_response(
    user: &str,
    password: &str,
    method: &str,
    uri: &str,
    digest: &Digest,
) -> String {
    let ha1 = md5_hex(&format!("{user}:{}:{password}", digest.realm));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    md5_hex(&format!("{ha1}:{}:{ha2}", digest.nonce))
}

pub fn parse_digest(header: &str) -> Option<Digest> {
    let find = |key: &str| -> Option<String> {
        let start = header.find(key)? + key.len();
        let rest = &header[start..];
        let rest = rest.trim_start_matches(['=', ' ', '"']);
        let end = rest.find('"').unwrap_or(rest.len());
        Some(rest[..end].to_string())
    };
    Some(Digest {
        realm: find("realm=")?,
        nonce: find("nonce=")?,
    })
}

pub fn parse_sdp(body: &str) -> SdpInfo {
    let mut info = SdpInfo {
        video_codec: None,
        audio_codec: None,
        framerate: None,
        control: None,
    };
    for line in body.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("a=rtpmap:") {
            if let Some((_, codec)) = rest.split_once(' ') {
                let name = codec.split('/').next().unwrap_or(codec).to_uppercase();
                if info.video_codec.is_none() && line.contains("/90000") && !name.contains("MP4") {
                    info.video_codec = Some(name);
                } else if info.audio_codec.is_none() {
                    info.audio_codec = Some(name);
                }
            }
        } else if let Some(rest) = line.strip_prefix("a=framerate:") {
            info.framerate = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("a=control:") {
            if info.control.is_none() && rest.contains("trackID") {
                info.control = Some(rest.to_string());
            }
        }
    }
    info
}

fn request(method: &str, url: &str, cseq: u32, authorization: Option<&str>) -> String {
    let crlf = "\r\n";
    let mut out = String::new();
    out.push_str(&format!("{method} {url} RTSP/1.0{crlf}"));
    out.push_str(&format!("CSeq: {cseq}{crlf}"));
    out.push_str(&format!("User-Agent: proview-cctv-web/0.1{crlf}"));
    out.push_str(&format!("Accept: application/sdp{crlf}"));
    if let Some(auth) = authorization {
        out.push_str(auth);
        out.push_str(crlf);
    }
    out.push_str(crlf);
    out
}

fn split_response(raw: &str) -> (String, String) {
    let separator = "\r\n\r\n";
    match raw.split_once(separator) {
        Some((head, body)) => (head.to_string(), body.to_string()),
        None => (raw.to_string(), String::new()),
    }
}
fn header_value(head: &str, name: &str) -> Option<String> {
    head.lines()
        .find(|line| {
            line.to_ascii_lowercase()
                .starts_with(&name.to_ascii_lowercase())
        })
        .and_then(|line| line.split_once(':'))
        .map(|(_, value)| value.trim().to_string())
}

async fn exchange(stream: &mut TcpStream, payload: String) -> Result<String, RtspError> {
    stream
        .write_all(payload.as_bytes())
        .await
        .map_err(RtspError::Io)?;
    let mut buffer = Vec::new();
    loop {
        let mut chunk = [0u8; 4096];
        let read = match tokio::time::timeout(Duration::from_millis(1500), stream.read(&mut chunk))
            .await
        {
            Ok(Ok(read)) => read,
            Ok(Err(error)) => return Err(RtspError::Io(error)),
            Err(_) => break,
        };
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8_lossy(&buffer);
        if text.contains(
            "

",
        ) {
            if let Some(len) = header_value(&text, "content-length") {
                let (head, body) = split_response(&text);
                if let Ok(expected) = len.parse::<usize>() {
                    if body.len() >= expected {
                        return Ok(text.to_string());
                    }
                }
                let _ = head;
            } else {
                return Ok(text.to_string());
            }
        }
    }
    Ok(String::from_utf8_lossy(&buffer).to_string())
}

pub async fn describe(
    host: &str,
    port: u16,
    path: &str,
    user: &str,
    password: &str,
) -> Result<SdpInfo, RtspError> {
    match tokio::time::timeout(
        Duration::from_secs(6),
        describe_inner(host, port, path, user, password),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(RtspError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "rtsp exchange timed out",
        ))),
    }
}

async fn describe_inner(
    host: &str,
    port: u16,
    path: &str,
    user: &str,
    password: &str,
) -> Result<SdpInfo, RtspError> {
    let url = format!("rtsp://{host}:{port}{path}");
    let mut stream = tokio::time::timeout(Duration::from_secs(8), TcpStream::connect((host, port)))
        .await
        .map_err(|_| {
            RtspError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "connect timed out",
            ))
        })?
        .map_err(RtspError::Io)?;

    let first = exchange(&mut stream, request("DESCRIBE", &url, 1, None)).await?;
    let (head, _) = split_response(&first);
    if head.starts_with("RTSP/1.0 401") {
        let challenge = header_value(&head, "www-authenticate")
            .ok_or_else(|| RtspError::Parse("401 without challenge".to_string()))?;
        let digest = parse_digest(&challenge).ok_or(RtspError::Unauthorized)?;
        let response = digest_response(user, password, "DESCRIBE", &url, &digest);
        let auth = format!(
            "Authorization: Digest username=\"{user}\", realm=\"{}\", nonce=\"{}\", uri=\"{url}\", response=\"{response}\"",
            digest.realm, digest.nonce
        );
        let second = exchange(&mut stream, request("DESCRIBE", &url, 2, Some(&auth))).await?;
        let (head2, body2) = split_response(&second);
        if !head2.starts_with("RTSP/1.0 200") {
            let status = head2.lines().next().unwrap_or("").to_string();
            if status.contains("401") || status.contains("403") {
                return Err(RtspError::Unauthorized);
            }
            return Err(RtspError::Status(status));
        }
        return Ok(parse_sdp(&body2));
    }

    let (head_ok, body) = split_response(&first);
    if !head_ok.starts_with("RTSP/1.0 200") {
        return Err(RtspError::Status(
            head_ok.lines().next().unwrap_or("").to_string(),
        ));
    }
    Ok(parse_sdp(&body))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub kind: MediaKind,
    pub payload_type: u8,
    pub codec: String,
    pub control: Option<String>,
    pub framerate: Option<u32>,
    pub vps: Option<Vec<u8>>,
    pub sps: Option<Vec<u8>>,
    pub pps: Option<Vec<u8>>,
}

fn decode_parameter(value: &str) -> Option<Vec<u8>> {
    let trimmed = value.trim().trim_end_matches(';').trim_matches('"');
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(trimmed) {
        if !bytes.is_empty() {
            return Some(bytes);
        }
    }
    if trimmed.len() % 2 == 0 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut out = Vec::with_capacity(trimmed.len() / 2);
        for index in (0..trimmed.len()).step_by(2) {
            out.push(u8::from_str_radix(&trimmed[index..index + 2], 16).ok()?);
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    None
}

pub fn parse_tracks(sdp: &str) -> Vec<Track> {
    let mut tracks: Vec<Track> = Vec::new();
    for line in sdp.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("m=") {
            let mut parts = rest.split_whitespace();
            let kind = match parts.next() {
                Some("video") => MediaKind::Video,
                Some("audio") => MediaKind::Audio,
                _ => continue,
            };
            let _port = parts.next();
            let _protocol = parts.next();
            let payload_type = parts
                .next()
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or(0);
            tracks.push(Track {
                kind,
                payload_type,
                codec: String::new(),
                control: None,
                framerate: None,
                vps: None,
                sps: None,
                pps: None,
            });
            continue;
        }
        let Some(track) = tracks.last_mut() else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("a=rtpmap:") {
            if let Some((payload_type, encoding)) = rest.split_once(' ') {
                let matches = payload_type.trim().parse::<u8>().ok() == Some(track.payload_type);
                if matches && track.codec.is_empty() {
                    let name = encoding.split('/').next().unwrap_or(encoding);
                    track.codec = name.trim().to_uppercase();
                }
            }
        } else if let Some(rest) = line.strip_prefix("a=fmtp:") {
            let matches = rest
                .split_once(' ')
                .map(|(payload_type, params)| {
                    (
                        payload_type.trim().parse::<u8>().ok() == Some(track.payload_type),
                        params,
                    )
                })
                .unwrap_or((false, ""));
            if matches.0 {
                for item in matches.1.split(';') {
                    let Some((key, value)) = item.split_once('=') else {
                        continue;
                    };
                    match key.trim() {
                        "sprop-vps" => track.vps = decode_parameter(value),
                        "sprop-sps" => track.sps = decode_parameter(value),
                        "sprop-pps" => track.pps = decode_parameter(value),
                        _ => {}
                    }
                }
            }
        } else if let Some(rest) = line.strip_prefix("a=control:") {
            let control = rest.trim();
            if !control.is_empty() && control != "*" {
                track.control = Some(control.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("a=framerate:") {
            track.framerate = rest.trim().parse().ok();
        }
    }
    tracks
}

fn parse_interleaved(value: &str) -> Option<(u8, u8)> {
    let rest = value.split("interleaved=").nth(1)?;
    let pair = rest.split(';').next()?;
    let (first, second) = pair.split_once('-')?;
    Some((first.trim().parse().ok()?, second.trim().parse().ok()?))
}

struct Reader {
    stream: TcpStream,
    buffer: Vec<u8>,
}

impl Reader {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            buffer: Vec::with_capacity(32 * 1024),
        }
    }

    async fn more(&mut self) -> Result<(), RtspError> {
        let mut chunk = [0u8; 8192];
        let read = tokio::time::timeout(Duration::from_secs(10), self.stream.read(&mut chunk))
            .await
            .map_err(|_| {
                RtspError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "rtsp read timed out",
                ))
            })?
            .map_err(RtspError::Io)?;
        if read == 0 {
            return Err(RtspError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "peer closed the rtsp stream",
            )));
        }
        self.buffer.extend_from_slice(&chunk[..read]);
        Ok(())
    }

    async fn need(&mut self, length: usize) -> Result<(), RtspError> {
        while self.buffer.len() < length {
            self.more().await?;
        }
        Ok(())
    }

    async fn response(&mut self) -> Result<(String, String), RtspError> {
        loop {
            let separator = self
                .buffer
                .windows(4)
                .position(|window| window == b"\r\n\r\n");
            let Some(position) = separator else {
                self.more().await?;
                continue;
            };
            let head = String::from_utf8_lossy(&self.buffer[..position]).to_string();
            let length = header_value(&head, "content-length")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let total = position + 4 + length;
            self.need(total).await?;
            let body = String::from_utf8_lossy(&self.buffer[position + 4..total]).to_string();
            self.buffer.drain(..total);
            return Ok((head, body));
        }
    }

    async fn interleaved(&mut self) -> Result<(u8, Vec<u8>), RtspError> {
        loop {
            if self.buffer.is_empty() {
                self.more().await?;
            }
            if self.buffer[0] == b'$' {
                self.need(4).await?;
                let channel = self.buffer[1];
                let length = u16::from_be_bytes([self.buffer[2], self.buffer[3]]) as usize;
                self.need(4 + length).await?;
                let payload = self.buffer[4..4 + length].to_vec();
                self.buffer.drain(..4 + length);
                return Ok((channel, payload));
            }
            if self.buffer.starts_with(b"RTSP/") {
                let _ = self.response().await?;
                continue;
            }
            match self.buffer.iter().position(|byte| *byte == b'$') {
                Some(index) if index > 0 => {
                    self.buffer.drain(..index);
                }
                Some(_) => {}
                None => {
                    let dropped = self.buffer.len();
                    self.buffer.clear();
                    tracing::warn!(
                        dropped,
                        "discarded bytes that were not interleaved rtsp data"
                    );
                }
            }
        }
    }
}

pub struct RtspSession {
    reader: Reader,
    url: String,
    cseq: u32,
    user: String,
    password: String,
    digest: Option<Digest>,
    session: Option<String>,
    tracks: Vec<Track>,
    video: Option<usize>,
    interleaved: Option<(u8, u8)>,
}

impl RtspSession {
    pub async fn open(
        host: &str,
        port: u16,
        path: &str,
        user: &str,
        password: &str,
    ) -> Result<Self, RtspError> {
        let stream = tokio::time::timeout(Duration::from_secs(8), TcpStream::connect((host, port)))
            .await
            .map_err(|_| {
                RtspError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "rtsp connect timed out",
                ))
            })?
            .map_err(RtspError::Io)?;
        let _ = stream.set_nodelay(true);
        let mut session = Self {
            reader: Reader::new(stream),
            url: format!("rtsp://{host}:{port}{path}"),
            cseq: 0,
            user: user.to_string(),
            password: password.to_string(),
            digest: None,
            session: None,
            tracks: Vec::new(),
            video: None,
            interleaved: None,
        };
        let url = session.url.clone();
        let (head, body) = session
            .request("DESCRIBE", url, &["Accept: application/sdp".to_string()])
            .await?;
        let status = head.lines().next().unwrap_or("").to_string();
        if !status.contains(" 200") {
            return Err(RtspError::Status(status));
        }
        session.tracks = parse_tracks(&body);
        session.video = session
            .tracks
            .iter()
            .position(|track| track.kind == MediaKind::Video);
        Ok(session)
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn video_track(&self) -> Option<&Track> {
        self.video.and_then(|index| self.tracks.get(index))
    }

    pub fn video_channel(&self) -> Option<u8> {
        self.interleaved.map(|pair| pair.0)
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session.as_deref()
    }

    pub async fn setup_video(&mut self, requested: (u8, u8)) -> Result<(u8, u8), RtspError> {
        let index = self
            .video
            .ok_or_else(|| RtspError::Parse("sdp has no video track".to_string()))?;
        let track_url = match self.tracks[index].control.clone() {
            Some(control) if control.starts_with("rtsp://") => control,
            Some(control) => format!("{}/{}", self.url.trim_end_matches('/'), control),
            None => self.url.clone(),
        };
        let transport = format!(
            "Transport: RTP/AVP/TCP;unicast;interleaved={}-{}",
            requested.0, requested.1
        );
        let (head, _) = self.request("SETUP", track_url, &[transport]).await?;
        let status = head.lines().next().unwrap_or("").to_string();
        if !status.contains(" 200") {
            return Err(RtspError::Status(status));
        }
        let assigned = header_value(&head, "transport")
            .and_then(|value| parse_interleaved(&value))
            .unwrap_or(requested);
        self.interleaved = Some(assigned);
        Ok(assigned)
    }

    pub async fn play(&mut self) -> Result<(), RtspError> {
        let url = self.url.clone();
        let (head, _) = self
            .request("PLAY", url, &["Range: npt=0.000-".to_string()])
            .await?;
        let status = head.lines().next().unwrap_or("").to_string();
        if !status.contains(" 200") {
            return Err(RtspError::Status(status));
        }
        Ok(())
    }

    pub async fn next_packet(&mut self) -> Result<(u8, Vec<u8>), RtspError> {
        self.reader.interleaved().await
    }

    pub async fn teardown(&mut self) -> Result<(), RtspError> {
        let url = self.url.clone();
        let (head, _) = self.request("TEARDOWN", url, &[]).await?;
        let status = head.lines().next().unwrap_or("").to_string();
        if !status.contains(" 200") {
            return Err(RtspError::Status(status));
        }
        Ok(())
    }

    async fn request(
        &mut self,
        method: &str,
        url: String,
        headers: &[String],
    ) -> Result<(String, String), RtspError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            self.cseq += 1;
            let mut payload = format!(
                "{method} {url} RTSP/1.0\r\nCSeq: {}\r\nUser-Agent: proview-cctv-web/0.1\r\n",
                self.cseq
            );
            for header in headers {
                payload.push_str(header);
                payload.push_str("\r\n");
            }
            if let Some(session) = &self.session {
                payload.push_str(&format!("Session: {session}\r\n"));
            }
            if let Some(digest) = &self.digest {
                let response = digest_response(&self.user, &self.password, method, &url, digest);
                payload.push_str(&format!(
                    "Authorization: Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{url}\", response=\"{response}\"\r\n",
                    self.user, digest.realm, digest.nonce
                ));
            }
            payload.push_str("\r\n");
            self.reader
                .stream
                .write_all(payload.as_bytes())
                .await
                .map_err(RtspError::Io)?;
            let (head, body) = self.reader.response().await?;
            let status = head.lines().next().unwrap_or("").to_string();
            if status.contains(" 401") && self.digest.is_none() && attempt == 1 {
                let challenge = header_value(&head, "www-authenticate")
                    .ok_or_else(|| RtspError::Parse("401 without a challenge".to_string()))?;
                self.digest = Some(parse_digest(&challenge).ok_or(RtspError::Unauthorized)?);
                continue;
            }
            if status.contains(" 401") || status.contains(" 403") {
                return Err(RtspError::Unauthorized);
            }
            if let Some(id) = header_value(&head, "session") {
                let trimmed = id.split(';').next().unwrap_or(&id).trim().to_string();
                if !trimmed.is_empty() {
                    self.session = Some(trimmed);
                }
            }
            return Ok((head, body));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_response_follows_ha1_nonce_ha2() {
        let digest = Digest {
            realm: "testrealm@host.com".into(),
            nonce: "dcd98b7102dd2f0e8b11d0f600bfb0c093".into(),
        };
        let response = digest_response(
            "Mufasa",
            "Circle Of Life",
            "GET",
            "/dir/index.html",
            &digest,
        );
        assert_eq!(response, "670fd8c2df070c60b045671b8b24ff02");
    }

    #[test]
    fn challenge_parser_reads_realm_and_nonce() {
        let header =
            r#"Digest realm="b891e7cc20b4a341", nonce="a1b2c3", stale="FALSE", Basic realm="x""#;
        let digest = parse_digest(header).expect("digest");
        assert_eq!(digest.realm, "b891e7cc20b4a341");
        assert_eq!(digest.nonce, "a1b2c3");
    }

    #[test]
    fn sdp_parser_picks_hevc_video_and_pcma_audio() {
        let sdp = "v=0\r\nm=video 0 RTP/AVP 98\r\na=rtpmap:98 H265/90000\r\na=framerate:25\r\na=control:trackID=3\r\nm=audio 0 RTP/AVP 8\r\na=rtpmap:8 PCMA/8000\r\n";
        let info = parse_sdp(sdp);
        assert_eq!(info.video_codec.as_deref(), Some("H265"));
        assert_eq!(info.audio_codec.as_deref(), Some("PCMA"));
        assert_eq!(info.framerate, Some(25));
        assert_eq!(info.control.as_deref(), Some("trackID=3"));
    }

    const DEVICE_SDP: &str = "v=0\r\no=- 1 1 IN IP4 0.0.0.0\r\ns=RTSP Server\r\na=control:*\r\nm=video 0 RTP/AVP 98\r\na=rtpmap:98 H265/90000 \r\na=fmtp:98 profile-id=010101;sprop-pps=RAHA88DhRSQ=;sprop-sps=QgEBAWAAAAMAAAMAAAMAAAMAP6ABICACiH+curwS4IA=;sprop-vps=QAEMAf//AWAAAAMAAAMAAAMAAAMAPywMAAAPoAAAu4FA;\r\na=framerate:25\r\na=control:trackID=3\r\nm=audio 0 RTP/AVP 8\r\na=rtpmap:8 PCMA/8000\r\na=control:trackID=4\r\n";

    #[test]
    fn tracks_are_split_per_media_section_with_parameter_sets() {
        let tracks = parse_tracks(DEVICE_SDP);
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].kind, MediaKind::Video);
        assert_eq!(tracks[0].codec, "H265");
        assert_eq!(tracks[0].payload_type, 98);
        assert_eq!(tracks[0].framerate, Some(25));
        assert_eq!(tracks[0].control.as_deref(), Some("trackID=3"));
        assert_eq!(
            crate::media::hevc::nal_type(&tracks[0].vps.clone().unwrap()),
            Some(32)
        );
        assert_eq!(
            crate::media::hevc::nal_type(&tracks[0].sps.clone().unwrap()),
            Some(33)
        );
        assert_eq!(
            crate::media::hevc::nal_type(&tracks[0].pps.clone().unwrap()),
            Some(34)
        );
        assert_eq!(tracks[1].kind, MediaKind::Audio);
        assert_eq!(tracks[1].codec, "PCMA");
        assert_eq!(tracks[1].control.as_deref(), Some("trackID=4"));
    }

    #[test]
    fn interleaved_pairs_are_read_from_the_transport_header() {
        let value = "RTP/AVP/TCP;unicast;interleaved=2-3;mode=PLAY";
        assert_eq!(parse_interleaved(value), Some((2, 3)));
        assert_eq!(parse_interleaved("RTP/AVP;unicast"), None);
    }

    async fn read_request(socket: &mut TcpStream) -> String {
        let mut buffer = Vec::new();
        loop {
            let mut chunk = [0u8; 1024];
            let read = socket.read(&mut chunk).await.unwrap();
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&buffer).to_string()
    }

    #[tokio::test]
    async fn a_session_authenticates_setups_and_reads_interleaved_media() {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let first = read_request(&mut socket).await;
            assert!(first.starts_with("DESCRIBE "));
            assert!(!first.contains("Authorization:"));
            socket
                .write_all(
                    b"RTSP/1.0 401 Unauthorized\r\nCSeq: 1\r\nWWW-Authenticate: Digest realm=\"realm1\", nonce=\"abc\"\r\n\r\n",
                )
                .await
                .unwrap();
            let second = read_request(&mut socket).await;
            assert!(second.starts_with("DESCRIBE "));
            assert!(second.contains("Authorization: Digest"));
            let body = DEVICE_SDP;
            let response = format!(
                "RTSP/1.0 200 OK\r\nCSeq: 2\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            let setup = read_request(&mut socket).await;
            assert!(setup.starts_with("SETUP rtsp://"), "{setup}");
            assert!(setup.contains("/trackID=3"), "{setup}");
            assert!(setup.contains("interleaved=0-1"), "{setup}");
            assert!(setup.contains("Authorization: Digest"), "{setup}");
            socket
                .write_all(
                    b"RTSP/1.0 200 OK\r\nCSeq: 3\r\nSession: 1234;timeout=60\r\nTransport: RTP/AVP/TCP;unicast;interleaved=2-3;mode=PLAY\r\n\r\n",
                )
                .await
                .unwrap();
            let play = read_request(&mut socket).await;
            assert!(play.starts_with("PLAY "), "{play}");
            assert!(play.contains("Session: 1234"), "{play}");
            socket
                .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 4\r\nSession: 1234\r\n\r\n")
                .await
                .unwrap();
            let mut rtp = vec![0x80, 98, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
            rtp.extend_from_slice(&[0x26, 0x01, 0xaf]);
            let mut frame = vec![b'$', 2];
            frame.extend_from_slice(&(rtp.len() as u16).to_be_bytes());
            frame.extend_from_slice(&rtp);
            socket.write_all(&frame).await.unwrap();
            let teardown = read_request(&mut socket).await;
            assert!(teardown.starts_with("TEARDOWN "), "{teardown}");
            socket
                .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 5\r\n\r\n")
                .await
                .unwrap();
        });

        let host = addr.ip().to_string();
        let mut session = RtspSession::open(&host, addr.port(), "/", "admin", "secret")
            .await
            .unwrap();
        assert_eq!(
            session.video_track().map(|track| track.codec.as_str()),
            Some("H265")
        );
        assert_eq!(session.session_id(), None);
        assert_eq!(session.setup_video((0, 1)).await.unwrap(), (2, 3));
        assert_eq!(session.session_id(), Some("1234"));
        assert_eq!(session.video_channel(), Some(2));
        session.play().await.unwrap();
        let (channel, packet) = session.next_packet().await.unwrap();
        assert_eq!(channel, 2);
        let parsed = crate::media::rtp::parse(&packet).unwrap();
        assert_eq!(parsed.payload, &[0x26, 0x01, 0xaf]);
        session.teardown().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_second_401_after_the_digest_is_an_authorization_failure() {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            for cseq in 1..=2 {
                let _ = read_request(&mut socket).await;
                let response = format!(
                    "RTSP/1.0 401 Unauthorized\r\nCSeq: {cseq}\r\nWWW-Authenticate: Digest realm=\"realm1\", nonce=\"nonce{cseq}\"\r\n\r\n"
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let host = addr.ip().to_string();
        let result = RtspSession::open(&host, addr.port(), "/", "admin", "wrong").await;
        assert!(matches!(result, Err(RtspError::Unauthorized)));
    }
}
