use std::future::Future;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use super::client::CloudError;
use super::css::{CssClient, SignRequest, SignedFile, API_VERSION, HLS_REQUEST, VIDEO_OBJECT_TYPE};

pub const WINDOW_ENV: &str = "PROVIEW_CLOUD_CLIP_WINDOW_MS";
pub const DEFAULT_WINDOW: Duration = Duration::from_millis(7_000);
pub const TOTAL_ENV: &str = "PROVIEW_CLOUD_CLIP_TOTAL_MS";
pub const DEFAULT_TOTAL: Duration = Duration::from_secs(120);
pub const SIGN_BATCH: usize = 32;
pub const MAX_SEGMENTS: usize = 2_048;
pub const MAX_CLIP_BYTES: usize = 512 * 1024 * 1024;
pub const TS_SYNC: u8 = 0x47;

pub fn window() -> Duration {
    std::env::var(WINDOW_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value >= 50)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_WINDOW)
}

pub fn total() -> Duration {
    std::env::var(TOTAL_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value >= 50)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_TOTAL)
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub window: Duration,
    pub total: Duration,
}

impl Limits {
    pub fn from_env() -> Self {
        Self {
            window: window(),
            total: total(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Plan {
    Open,
    None,
    Expired,
    Unsupported,
    Unknown,
}

impl Plan {
    pub fn reason(self) -> &'static str {
        match self {
            Plan::Open => "This camera has a cloud storage plan.",
            Plan::None => "No cloud storage plan on this camera, so the vendor cloud keeps no video for it.",
            Plan::Expired => "The cloud storage plan of this camera has expired, so the vendor cloud keeps no new video for it.",
            Plan::Unsupported => "This camera does not support cloud video storage.",
            Plan::Unknown => "The vendor cloud did not say whether this camera has a cloud storage plan.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evidence {
    pub source: &'static str,
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanState {
    pub serial: String,
    pub plan: Plan,
    pub reason: String,
    pub source: &'static str,
    pub code: String,
    pub evidence: Vec<Evidence>,
}

pub const CAPS_SUPPORT: &str = "xmc.css.vid.support";
pub const CAPS_ENABLE: &str = "xmc.css.vid.enable";
pub const CAPS_EXPIRES: &str = "xmc.css.vid.expirationtime";
pub const CAPS_ENABLE_CHANNELS: &str = "xmc.css.vid.enable.channels";
pub const CAPS_EXPIRES_CHANNELS: &str = "xmc.css.vid.expirationtime.channels";

fn caps_flag(caps: &Value, key: &str) -> bool {
    match caps.get(key) {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => text.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn caps_number(caps: &Value, key: &str) -> i64 {
    match caps.get(key) {
        Some(Value::Number(number)) => number.as_i64().unwrap_or_default(),
        Some(Value::String(text)) => text.trim().parse().unwrap_or_default(),
        _ => 0,
    }
}

fn caps_list<'a>(caps: &'a Value, key: &str) -> Option<Vec<&'a str>> {
    caps.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| text.split('_').map(str::trim).collect())
}

pub fn plan_from_caps(caps: &Value, channel: u32, now: i64) -> (Plan, Evidence) {
    let detail = |enabled: bool, expires: i64| {
        format!(
            "{CAPS_SUPPORT}={} {CAPS_ENABLE}={enabled} {CAPS_EXPIRES}={expires}",
            caps_flag(caps, CAPS_SUPPORT)
        )
    };
    if caps.get(CAPS_SUPPORT).is_none() {
        return (
            Plan::Unknown,
            Evidence {
                source: "caps",
                code: "200".to_string(),
                detail: format!("the device capabilities carry no {CAPS_SUPPORT}"),
            },
        );
    }
    let (enabled, expires) = match (
        caps_list(caps, CAPS_ENABLE_CHANNELS),
        caps_list(caps, CAPS_EXPIRES_CHANNELS),
    ) {
        (Some(enabled), Some(expires))
            if enabled.len() == expires.len() && (channel as usize) < enabled.len() =>
        {
            (
                enabled[channel as usize] == "true",
                expires[channel as usize].parse().unwrap_or_default(),
            )
        }
        _ => (
            caps_flag(caps, CAPS_ENABLE),
            caps_number(caps, CAPS_EXPIRES),
        ),
    };
    let plan = if !caps_flag(caps, CAPS_SUPPORT) {
        Plan::Unsupported
    } else if enabled && expires > now {
        Plan::Open
    } else if expires > 0 && expires <= now {
        Plan::Expired
    } else {
        Plan::None
    };
    (
        plan,
        Evidence {
            source: "caps",
            code: "200".to_string(),
            detail: detail(enabled, expires),
        },
    )
}

pub fn hls_body(serial: &str, auth_code: &str, user_id: &str, day: &str, channel: u32) -> Value {
    json!({
        "CssCenter": {
            "Header": { "Version": API_VERSION, "MessageType": HLS_REQUEST },
            "Body": {
                "SerialNumber": serial.trim(),
                "AuthCode": auth_code.trim(),
                "UserId": user_id.trim(),
                "StartTime": format!("{day} 00:00:00"),
                "StopTime": format!("{day} 23:59:59"),
                "ForceRefresh": "0",
                "Channel": channel.to_string(),
            }
        }
    })
}

pub fn plan_from_css(reply: &Value) -> (Plan, Evidence) {
    let header = &reply["CssCenter"]["Header"];
    let code = match &header["ErrorNum"] {
        Value::String(text) => text.trim().to_string(),
        Value::Number(number) => number.to_string(),
        _ => String::new(),
    };
    let message = header["ErrorString"].as_str().unwrap_or_default().trim();
    let plan = match code.as_str() {
        "200" => Plan::Open,
        "403" => Plan::None,
        _ => Plan::Unknown,
    };
    (
        plan,
        Evidence {
            source: "css",
            code: if code.is_empty() {
                "none".to_string()
            } else {
                code
            },
            detail: if message.is_empty() {
                "the cloud storage centre answered without a reason".to_string()
            } else {
                message.to_string()
            },
        },
    )
}

pub fn decide(
    serial: &str,
    caps: Option<(Plan, Evidence)>,
    css: Option<(Plan, Evidence)>,
) -> PlanState {
    let chosen = match (&caps, &css) {
        (_, Some((Plan::Open, evidence))) => Some((Plan::Open, evidence.clone())),
        (Some((plan, evidence)), _) if *plan != Plan::Unknown => Some((*plan, evidence.clone())),
        (_, Some((plan, evidence))) if *plan != Plan::Unknown => Some((*plan, evidence.clone())),
        _ => None,
    };
    let evidence: Vec<Evidence> = caps
        .iter()
        .chain(css.iter())
        .map(|(_, evidence)| evidence.clone())
        .collect();
    let (plan, source, code) = match chosen {
        Some((plan, evidence)) => (plan, evidence.source, evidence.code),
        None => (Plan::Unknown, "none", String::new()),
    };
    PlanState {
        serial: serial.to_string(),
        plan,
        reason: plan.reason().to_string(),
        source,
        code,
        evidence,
    }
}

pub fn object_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && !name.starts_with('/')
        && !name.ends_with('/')
        && name
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || matches!(letter, '_' | '-' | '.' | '/'))
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub fn owned_object(name: &str, serial: &str) -> bool {
    let serial = serial.trim().to_ascii_lowercase();
    object_ok(name) && !serial.is_empty() && name.to_ascii_lowercase().contains(&serial)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Url(String),
    Object(String),
}

pub fn parse_playlist(text: &str, index_object: &str) -> Result<Vec<Segment>, String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    if lines.next() != Some("#EXTM3U") {
        return Err("the clip index is not an HLS playlist".to_string());
    }
    let prefix = index_object
        .rsplit_once('/')
        .map(|(head, _)| format!("{head}/"))
        .unwrap_or_default();
    let mut segments = Vec::new();
    for line in lines {
        if let Some(key) = line.strip_prefix("#EXT-X-KEY:") {
            if !key.to_ascii_uppercase().contains("METHOD=NONE") {
                return Err(format!(
                    "the clip is encrypted ({key}) and this build cannot decrypt it"
                ));
            }
            continue;
        }
        if line.starts_with("#EXT-X-STREAM-INF") {
            return Err("the clip index is a master playlist, not a clip".to_string());
        }
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with("http://") || line.starts_with("https://") {
            segments.push(Segment::Url(line.to_string()));
            continue;
        }
        let name = line.split(['?', '#']).next().unwrap_or_default();
        let object = format!("{prefix}{name}");
        if !object_ok(&object) {
            return Err(format!("the playlist names an unusable segment: {name}"));
        }
        segments.push(Segment::Object(object));
    }
    if segments.is_empty() {
        return Err("the playlist lists no segments".to_string());
    }
    if segments.len() > MAX_SEGMENTS {
        return Err(format!(
            "the playlist lists {} segments, more than {MAX_SEGMENTS}",
            segments.len()
        ));
    }
    Ok(segments)
}

pub fn url_file(address: &str) -> Result<SignedFile, String> {
    let url = reqwest::Url::parse(address)
        .map_err(|error| format!("the playlist names a bad segment url: {error}"))?;
    let host = match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_string(),
        _ => return Err("the playlist names a segment url without a host".to_string()),
    };
    let mut path = url.path().to_string();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    Ok(SignedFile {
        url: path,
        host,
        schema: url.scheme().to_string(),
        method: "GET".to_string(),
        date: String::new(),
        authorization: String::new(),
    })
}

pub fn pair_signed(objects: &[String], files: Vec<SignedFile>) -> Vec<(String, SignedFile)> {
    let named = |file: &SignedFile, object: &str| {
        let path = file
            .url
            .split('?')
            .next()
            .unwrap_or_default()
            .trim_start_matches('/');
        path == object || path.ends_with(&format!("/{object}"))
    };
    let by_name: Option<Vec<(String, SignedFile)>> = objects
        .iter()
        .map(|object| {
            files
                .iter()
                .find(|file| named(file, object))
                .map(|file| (object.clone(), file.clone()))
        })
        .collect();
    by_name.unwrap_or_else(|| objects.iter().cloned().zip(files).collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Sign,
    Playlist,
    Segment,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Stage::Sign => "sign",
            Stage::Playlist => "playlist",
            Stage::Segment => "segment",
        }
    }
}

impl std::fmt::Display for Stage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClipError {
    #[error("the cloud clip {stage} stage did not answer within {millis} ms")]
    Timeout { stage: Stage, millis: u128 },
    #[error("the cloud clip took longer than {millis} ms in total; it was at the {stage} stage, {done} of {segments} segments fetched")]
    Overall {
        stage: Stage,
        millis: u128,
        done: usize,
        segments: usize,
    },
    #[error("the cloud clip {stage} stage failed: {error}")]
    Cloud { stage: Stage, error: CloudError },
    #[error("{0}")]
    Playlist(String),
    #[error("the cloud clip is larger than {MAX_CLIP_BYTES} bytes")]
    TooLarge,
}

struct Budget {
    window: Duration,
    total: Duration,
    deadline: tokio::time::Instant,
    done: usize,
    segments: usize,
}

impl Budget {
    fn new(limits: Limits) -> Self {
        Self {
            window: limits.window,
            total: limits.total,
            deadline: tokio::time::Instant::now() + limits.total,
            done: 0,
            segments: 0,
        }
    }

    fn overall(&self, stage: Stage) -> ClipError {
        ClipError::Overall {
            stage,
            millis: self.total.as_millis(),
            done: self.done,
            segments: self.segments,
        }
    }

    async fn staged<T>(
        &self,
        stage: Stage,
        work: impl Future<Output = Result<T, CloudError>>,
    ) -> Result<T, ClipError> {
        let left = self
            .deadline
            .saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return Err(self.overall(stage));
        }
        let limit = left.min(self.window);
        match tokio::time::timeout(limit, work).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(ClipError::Cloud { stage, error }),
            Err(_) if left <= self.window => Err(self.overall(stage)),
            Err(_) => Err(ClipError::Timeout {
                stage,
                millis: self.window.as_millis(),
            }),
        }
    }
}

pub async fn fetch_clip(
    css: &CssClient,
    request: &SignRequest,
    limits: Limits,
) -> Result<Vec<u8>, ClipError> {
    let mut budget = Budget::new(limits);
    let index = budget
        .staged(
            Stage::Sign,
            css.sign_objects(
                request,
                VIDEO_OBJECT_TYPE,
                std::slice::from_ref(&request.object),
            ),
        )
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| ClipError::Playlist("the clip index was not signed".to_string()))?;
    let playlist = budget
        .staged(Stage::Playlist, css.fetch_object(&index))
        .await?;
    let segments = parse_playlist(&String::from_utf8_lossy(&playlist), &request.object)
        .map_err(ClipError::Playlist)?;
    budget.segments = segments.len();
    let objects: Vec<String> = segments
        .iter()
        .filter_map(|segment| match segment {
            Segment::Object(name) => Some(name.clone()),
            Segment::Url(_) => None,
        })
        .collect();
    let mut signed: Vec<(String, SignedFile)> = Vec::with_capacity(objects.len());
    for batch in objects.chunks(SIGN_BATCH) {
        let files = budget
            .staged(
                Stage::Sign,
                css.sign_objects(request, VIDEO_OBJECT_TYPE, batch),
            )
            .await?;
        signed.extend(pair_signed(batch, files));
    }
    let mut clip = Vec::new();
    for segment in &segments {
        let file = match segment {
            Segment::Url(address) => url_file(address).map_err(ClipError::Playlist)?,
            Segment::Object(name) => signed
                .iter()
                .find(|(object, _)| object == name)
                .map(|(_, file)| file.clone())
                .ok_or_else(|| ClipError::Playlist(format!("segment {name} was not signed")))?,
        };
        let bytes = budget
            .staged(Stage::Segment, css.fetch_object(&file))
            .await?;
        if bytes.first() != Some(&TS_SYNC) {
            return Err(ClipError::Cloud {
                stage: Stage::Segment,
                error: CloudError::Malformed("a segment is not MPEG-TS".to_string()),
            });
        }
        if clip.len() + bytes.len() > MAX_CLIP_BYTES {
            return Err(ClipError::TooLarge);
        }
        clip.extend_from_slice(&bytes);
        budget.done += 1;
    }
    Ok(clip)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_793_200_000;

    fn real_caps() -> Value {
        json!({
            "xmc.css.vid.support": true,
            "xmc.css.vid.enable": false,
            "xmc.css.vid.expirationtime": 0,
            "xmc.css.vid.enable.channels": "",
            "xmc.css.vid.expirationtime.channels": "",
            "xmc.css.pic.support": true
        })
    }

    #[test]
    fn the_measured_caps_of_a_camera_without_a_plan_read_as_none() {
        let (plan, evidence) = plan_from_caps(&real_caps(), 0, NOW);
        assert_eq!(plan, Plan::None);
        assert_eq!(evidence.source, "caps");
        assert!(
            evidence.detail.contains("xmc.css.vid.enable=false"),
            "{}",
            evidence.detail
        );
    }

    #[test]
    fn caps_map_support_enable_and_expiry_like_the_app() {
        let mut caps = real_caps();
        caps["xmc.css.vid.enable"] = json!(true);
        caps["xmc.css.vid.expirationtime"] = json!(NOW + 3600);
        assert_eq!(plan_from_caps(&caps, 0, NOW).0, Plan::Open);
        caps["xmc.css.vid.expirationtime"] = json!((NOW - 1).to_string());
        assert_eq!(plan_from_caps(&caps, 0, NOW).0, Plan::Expired);
        caps["xmc.css.vid.support"] = json!(false);
        assert_eq!(plan_from_caps(&caps, 0, NOW).0, Plan::Unsupported);
        assert_eq!(plan_from_caps(&json!({}), 0, NOW).0, Plan::Unknown);
    }

    #[test]
    fn channel_lists_override_the_device_flags_by_index() {
        let mut caps = real_caps();
        caps["xmc.css.vid.enable"] = json!(true);
        caps["xmc.css.vid.expirationtime"] = json!(NOW + 3600);
        caps["xmc.css.vid.enable.channels"] = json!("false_true");
        caps["xmc.css.vid.expirationtime.channels"] = json!(format!("0_{}", NOW + 60));
        assert_eq!(plan_from_caps(&caps, 0, NOW).0, Plan::None);
        assert_eq!(plan_from_caps(&caps, 1, NOW).0, Plan::Open);
        assert_eq!(plan_from_caps(&caps, 5, NOW).0, Plan::Open);
    }

    #[test]
    fn css_403_is_no_plan_and_other_codes_say_nothing() {
        let refused = json!({"CssCenter":{"Header":{"ErrorString":"Dev Not Open Cloud Storage","MessageType":"MSG_GET_CSS_HLS_RSP","ErrorNum":"403"}}});
        let (plan, evidence) = plan_from_css(&refused);
        assert_eq!(plan, Plan::None);
        assert_eq!(evidence.code, "403");
        assert_eq!(evidence.detail, "Dev Not Open Cloud Storage");
        let open = json!({"CssCenter":{"Header":{"ErrorNum":200}}});
        assert_eq!(plan_from_css(&open).0, Plan::Open);
        let odd =
            json!({"CssCenter":{"Header":{"ErrorNum":"405","ErrorString":"restricted area"}}});
        assert_eq!(plan_from_css(&odd).0, Plan::Unknown);
    }

    #[test]
    fn hls_body_asks_for_the_whole_day_on_the_channel() {
        let body = hls_body("SN0001", "auth", "user", "2026-10-29", 1);
        let inner = &body["CssCenter"]["Body"];
        assert_eq!(
            body["CssCenter"]["Header"]["MessageType"],
            "MSG_GET_CSS_HLS_REQ"
        );
        assert_eq!(inner["StartTime"], "2026-10-29 00:00:00");
        assert_eq!(inner["StopTime"], "2026-10-29 23:59:59");
        assert_eq!(inner["Channel"], "1");
        assert_eq!(inner["ForceRefresh"], "0");
    }

    #[test]
    fn decide_prefers_a_served_index_then_caps_then_css() {
        let caps_none = plan_from_caps(&real_caps(), 0, NOW);
        let css_open = (
            Plan::Open,
            Evidence {
                source: "css",
                code: "200".into(),
                detail: "ok".into(),
            },
        );
        let css_refused = (
            Plan::None,
            Evidence {
                source: "css",
                code: "403".into(),
                detail: "Dev Not Open Cloud Storage".into(),
            },
        );
        let caps_unknown = (
            Plan::Unknown,
            Evidence {
                source: "caps",
                code: "200".into(),
                detail: "-".into(),
            },
        );
        assert_eq!(
            decide("s", Some(caps_none.clone()), Some(css_open)).plan,
            Plan::Open
        );
        let state = decide("s", Some(caps_none), Some(css_refused.clone()));
        assert_eq!((state.plan, state.source), (Plan::None, "caps"));
        assert_eq!(state.evidence.len(), 2);
        let state = decide("s", Some(caps_unknown), Some(css_refused.clone()));
        assert_eq!(
            (state.plan, state.source, state.code.as_str()),
            (Plan::None, "css", "403")
        );
        let state = decide("s", None, None);
        assert_eq!(state.plan, Plan::Unknown);
        assert!(state.reason.contains("did not say"));
    }

    #[test]
    fn object_names_refuse_traversal_and_foreign_serials() {
        assert!(owned_object("SN0001/20261029/index.m3u8", "sn0001"));
        assert!(!owned_object("SN0002/index.m3u8", "SN0001"));
        assert!(!owned_object("SN0001/../x.m3u8", "SN0001"));
        assert!(!owned_object("/SN0001/x.m3u8", "SN0001"));
        assert!(!owned_object("SN0001//x.m3u8", "SN0001"));
        assert!(!owned_object("SN0001/x y.m3u8", "SN0001"));
        assert!(!owned_object("SN0001/x.m3u8", ""));
    }

    #[test]
    fn playlist_segments_resolve_beside_the_index() {
        let text = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXTINF:10.0,\nseg0.ts\n#EXTINF:4.0,\nseg1.ts?x=1\nhttps://h.aliyuncs.com/p/seg2.ts?sig=a\n#EXT-X-ENDLIST\n";
        let segments = parse_playlist(text, "SN0001/20261029/index.m3u8").unwrap();
        assert_eq!(
            segments,
            vec![
                Segment::Object("SN0001/20261029/seg0.ts".into()),
                Segment::Object("SN0001/20261029/seg1.ts".into()),
                Segment::Url("https://h.aliyuncs.com/p/seg2.ts?sig=a".into()),
            ]
        );
        assert_eq!(
            parse_playlist("#EXTM3U\nseg.ts\n", "index.m3u8").unwrap(),
            vec![Segment::Object("seg.ts".into())]
        );
    }

    #[test]
    fn playlists_that_cannot_be_served_say_why() {
        assert!(parse_playlist("<Error>AccessDenied</Error>", "a/i.m3u8")
            .unwrap_err()
            .contains("not an HLS"));
        assert!(parse_playlist("#EXTM3U\n#EXT-X-ENDLIST\n", "a/i.m3u8")
            .unwrap_err()
            .contains("no segments"));
        assert!(parse_playlist(
            "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\nseg.ts\n",
            "a/i.m3u8"
        )
        .unwrap_err()
        .contains("encrypted"));
        assert!(parse_playlist("#EXTM3U\n#EXT-X-KEY:METHOD=NONE\nseg.ts\n", "a/i.m3u8").is_ok());
        assert!(parse_playlist("#EXTM3U\n../../etc/passwd\n", "a/i.m3u8")
            .unwrap_err()
            .contains("unusable"));
        assert!(parse_playlist(
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nlow.m3u8\n",
            "a/i.m3u8"
        )
        .unwrap_err()
        .contains("master"));
    }

    #[test]
    fn signed_entries_pair_by_name_before_order() {
        let file = |url: &str| SignedFile {
            url: url.to_string(),
            ..SignedFile::default()
        };
        let objects = vec!["a/one.ts".to_string(), "a/two.ts".to_string()];
        let paired = pair_signed(&objects, vec![file("/a/two.ts"), file("/a/one.ts?s=1")]);
        assert_eq!(paired[0].1.url, "/a/one.ts?s=1");
        assert_eq!(paired[1].1.url, "/a/two.ts");
        let ordered = pair_signed(&objects, vec![file("/x"), file("/y")]);
        assert_eq!(ordered[1].1.url, "/y");
    }

    #[test]
    fn segment_urls_become_plain_requests() {
        let file = url_file("http://127.0.0.1:8080/hls/a/b/1_2.ts?k=v").unwrap();
        assert_eq!(file.address(), "http://127.0.0.1:8080/hls/a/b/1_2.ts?k=v");
        assert!(file.authorization.is_empty());
    }

    fn slow_cloud(segments: usize, pause: Duration) -> u16 {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                std::thread::spawn(move || {
                    let mut buffer = Vec::new();
                    let mut chunk = [0u8; 8192];
                    loop {
                        let read = stream.read(&mut chunk).unwrap_or(0);
                        if read == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..read]);
                        let text = String::from_utf8_lossy(&buffer).to_string();
                        let Some(split) = text.find("\r\n\r\n") else {
                            continue;
                        };
                        let length = text[..split]
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        if buffer.len() < split + 4 + length {
                            continue;
                        }
                        let path = text.split(' ').nth(1).unwrap_or("/").to_string();
                        let (kind, body): (&str, Vec<u8>) = if text.starts_with("POST") {
                            let request: Value =
                                serde_json::from_str(&text[split + 4..split + 4 + length])
                                    .unwrap_or_default();
                            let info: Vec<Value> = request["CssCenter"]["Body"]["ObjInfo"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default()
                                .iter()
                                .map(|entry| {
                                    let name = entry["ObjName"].as_str().unwrap_or_default();
                                    json!({"Schema":"http","Host":format!("127.0.0.1:{port}"),"URL":format!("/{name}"),"ReqHeader":{"Date":"d","Authorization":"a"}})
                                })
                                .collect();
                            (
                                "application/json",
                                json!({"CssCenter":{"Header":{"ErrorNum":"200","MessageType":"MSG_DOWNLOAD_SIGN_RSP"},"Body":{"Method":"GET","ObjInfo":info}}})
                                    .to_string()
                                    .into_bytes(),
                            )
                        } else if path.ends_with(".m3u8") {
                            let lines: String = (0..segments)
                                .map(|index| format!("#EXTINF:2.0,\ns{index}.ts\n"))
                                .collect();
                            (
                                "application/vnd.apple.mpegurl",
                                format!(
                                    "#EXTM3U\n#EXT-X-TARGETDURATION:2\n{lines}#EXT-X-ENDLIST\n"
                                )
                                .into_bytes(),
                            )
                        } else {
                            std::thread::sleep(pause);
                            ("video/mp2t", vec![TS_SYNC; 188])
                        };
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(&body);
                        return;
                    }
                });
            }
        });
        port
    }

    fn slow_request() -> SignRequest {
        SignRequest {
            serial: "SN0001".into(),
            object: "SN0001/day/index.m3u8".into(),
            bucket: "OSS_v".into(),
            ..SignRequest::default()
        }
    }

    #[tokio::test]
    async fn a_cloud_that_answers_each_call_inside_the_window_still_meets_the_total_deadline() {
        let port = slow_cloud(40, Duration::from_millis(120));
        let css = CssClient::new(format!("127.0.0.1:{port}"));
        let started = std::time::Instant::now();
        let error = fetch_clip(
            &css,
            &slow_request(),
            Limits {
                window: Duration::from_millis(400),
                total: Duration::from_millis(900),
            },
        )
        .await
        .unwrap_err();
        let elapsed = started.elapsed();
        match &error {
            ClipError::Overall {
                stage,
                millis,
                done,
                segments,
            } => {
                assert_eq!(*stage, Stage::Segment);
                assert_eq!(*millis, 900);
                assert_eq!(*segments, 40);
                assert!(*done > 0 && *done < 40, "{done}");
            }
            other => panic!("expected the total deadline, got {other}"),
        }
        assert!(elapsed >= Duration::from_millis(850), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(1600), "{elapsed:?}");
        assert!(error.to_string().contains("segment stage"), "{error}");
    }

    #[tokio::test]
    async fn the_same_slow_cloud_finishes_when_the_total_allows_it() {
        let port = slow_cloud(3, Duration::from_millis(120));
        let css = CssClient::new(format!("127.0.0.1:{port}"));
        let clip = fetch_clip(
            &css,
            &slow_request(),
            Limits {
                window: Duration::from_millis(400),
                total: Duration::from_secs(10),
            },
        )
        .await
        .expect("clip");
        assert_eq!(clip.len(), 3 * 188);
    }

    #[tokio::test]
    async fn a_silent_sign_centre_is_a_sign_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let held: Vec<_> = listener.incoming().take(1).collect();
            std::thread::sleep(Duration::from_secs(3));
            drop(held);
        });
        let css = CssClient::new(format!("127.0.0.1:{port}"));
        let request = SignRequest {
            serial: "SN0001".into(),
            object: "SN0001/i.m3u8".into(),
            bucket: "OSS_v".into(),
            ..SignRequest::default()
        };
        let started = std::time::Instant::now();
        let error = fetch_clip(
            &css,
            &request,
            Limits {
                window: Duration::from_millis(300),
                total: Duration::from_secs(30),
            },
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                error,
                ClipError::Timeout {
                    stage: Stage::Sign,
                    millis: 300
                }
            ),
            "{error}"
        );
        assert!(started.elapsed() < Duration::from_millis(1500));
        assert!(error.to_string().contains("sign stage"), "{error}");
    }
}
