use std::time::Duration;

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
}
