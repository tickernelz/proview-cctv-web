use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use proview_core::cloud::envelope::{EnvelopeClient, Identity};
use proview_core::cloud::status::{auth_code_of, StatusClient};
use proview_core::dvrip::crypto;
use proview_core::dvrip::frame::Header;
use proview_core::media::live::{start_body, LiveDemux, LIVE_MEDIA_ID};
use proview_core::provider::{p2p, rps};
use rand::Rng;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const LOGIN_ID: u16 = 1000;
const SYSTEM_INFO_ID: u16 = 1020;
const SYSTEM_INFO_REPLY: u16 = 1021;
const MONITOR_CLAIM: u16 = 0x0585;
const MONITOR_START: u16 = 0x0582;
const MONITOR_CHANNEL: u8 = 99;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Plaintext,
    Sealed,
    Replica,
}

impl Mode {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "plaintext" | "plain" => Some(Self::Plaintext),
            "sealed" | "key" => Some(Self::Sealed),
            "replica" => Some(Self::Replica),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Plaintext => "plaintext",
            Self::Sealed => "sealed",
            Self::Replica => "replica",
        }
    }
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

fn home_path(fallback: &str) -> PathBuf {
    PathBuf::from(env("HOME")).join(fallback)
}

fn secrets_dir() -> PathBuf {
    let explicit = env("PROVIEW_SECRETS_DIR");
    if explicit.is_empty() {
        home_path("re/proview-cctv/secrets")
    } else {
        PathBuf::from(explicit)
    }
}

fn oracle_dir() -> PathBuf {
    let explicit = env("PROVIEW_ORACLE_DIR");
    if explicit.is_empty() {
        home_path("re/proview-cctv/live-oracle")
    } else {
        PathBuf::from(explicit)
    }
}

fn load_env_file(path: &Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if key.is_empty() || std::env::var_os(key).is_some() {
            continue;
        }
        std::env::set_var(key, value);
    }
}

fn set_private_dir(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn restrict_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    fs::write(path, text)?;
    restrict_file(path);
    Ok(())
}

fn load_secrets() {
    let dir = secrets_dir();
    for name in ["proview.env", "device.env", "identity.env"] {
        load_env_file(&dir.join(name));
    }
}

fn random_hex(key: &str, bytes: usize) -> String {
    let explicit = env(key);
    if !explicit.is_empty() {
        return explicit;
    }
    let mut rng = rand::thread_rng();
    (0..bytes)
        .map(|_| format!("{:02x}", rng.gen::<u8>()))
        .collect()
}

fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { y + 1 } else { y }, month, day)
}

fn stamp(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    let (year, month, day) = civil((secs / 86_400) as i64);
    let rest = secs % 86_400;
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

fn hex_head(body: &[u8], width: usize) -> String {
    body.iter()
        .take(width)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

const SECRET_KEYS: [&str; 4] = ["PassWord", "UserName", "AdminToken", "CommunicateKey"];

fn redact(body: &[u8]) -> (String, bool) {
    let mut end = body.len();
    while end > 0 && matches!(body[end - 1], 0 | b'\n' | b'\r') {
        end -= 1;
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(&body[..end]) else {
        return (printable(body), false);
    };
    let mut hit = false;
    if let Some(map) = value.as_object_mut() {
        for key in SECRET_KEYS {
            let Some(entry) = map.get_mut(key) else {
                continue;
            };
            let width = entry.as_str().map(str::len).unwrap_or(0);
            *entry = Value::String(format!("<redacted:{width}>"));
            hit = true;
        }
    }
    if !hit {
        return (printable(body), false);
    }
    let text = serde_json::to_string(&value).unwrap_or_default();
    (text.chars().take(600).collect(), true)
}

fn printable(body: &[u8]) -> String {
    let mut end = body.len();
    while end > 0 && matches!(body[end - 1], 0 | b'\n' | b'\r') {
        end -= 1;
    }
    body[..end]
        .iter()
        .map(|byte| {
            let text = *byte as char;
            if text.is_ascii_graphic() || text == ' ' {
                text
            } else {
                '.'
            }
        })
        .take(600)
        .collect()
}

struct Trace {
    started: Instant,
    lines: Vec<String>,
    control: Vec<Value>,
    tally: BTreeMap<u16, (u64, u64)>,
}

impl Trace {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            lines: Vec::new(),
            control: Vec::new(),
            tally: BTreeMap::new(),
        }
    }

    fn at(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    fn record(&mut self, dir: &str, header: &Header, body: &[u8]) -> f64 {
        let at = self.at();
        let tally = self.tally.entry(header.id).or_insert((0, 0));
        tally.0 += 1;
        tally.1 += body.len() as u64;
        let mut head = hex_head(body, 32);
        let mut printed = String::new();
        let mut redacted = false;
        if header.id != LIVE_MEDIA_ID {
            let (text, secret) = redact(body);
            redacted = secret;
            if secret {
                head = hex_head(text.as_bytes(), 32);
            }
            printed = text;
        }
        self.lines.push(format!(
            "{at:>10.3} {dir} id=0x{:04x}({:<5}) ch={:<3} sess=0x{:08x} seq={:<5} end={} len={:<6} head={}",
            header.id,
            header.id,
            header.channel,
            header.session,
            header.sequence,
            u8::from(header.end),
            body.len(),
            head,
        ));
        if header.id != LIVE_MEDIA_ID {
            self.control.push(json!({
                "at_ms": (at * 1000.0).round() / 1000.0,
                "dir": dir,
                "id": header.id,
                "id_hex": format!("0x{:04x}", header.id),
                "channel": header.channel,
                "session": format!("0x{:08x}", header.session),
                "sequence": header.sequence,
                "end": header.end,
                "length": body.len(),
                "head32": head,
                "body": printed,
                "redacted": redacted,
            }));
        }
        at
    }

    fn close(&mut self, note: &str) {
        self.lines.push(format!("{:>10.3} -- {note}", self.at()));
    }
}

struct Wire {
    stream: TcpStream,
    session: u32,
    sequence: u32,
}

impl Wire {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            session: 0,
            sequence: 0,
        }
    }

    async fn send(
        &mut self,
        trace: &mut Trace,
        channel: u8,
        end: bool,
        id: u16,
        payload: &[u8],
    ) -> std::io::Result<Header> {
        self.sequence = self.sequence.wrapping_add(1);
        let header = Header {
            session: self.session,
            sequence: self.sequence,
            channel,
            end,
            id,
            length: payload.len() as u32,
        };
        trace.record(">>>", &header, payload);
        self.stream.write_all(&header.encode()).await?;
        self.stream.write_all(payload).await?;
        self.stream.flush().await?;
        Ok(header)
    }

    async fn send_json(
        &mut self,
        trace: &mut Trace,
        channel: u8,
        end: bool,
        id: u16,
        value: &Value,
        newline: bool,
    ) -> std::io::Result<Header> {
        let mut payload = serde_json::to_vec(value).map_err(std::io::Error::other)?;
        if newline {
            payload.push(b'\n');
        }
        payload.push(0);
        self.send(trace, channel, end, id, &payload).await
    }

    async fn recv(&mut self, trace: &mut Trace, budget: Duration) -> Option<(Header, Vec<u8>)> {
        let mut raw = [0u8; 20];
        tokio::time::timeout(budget, self.stream.read_exact(&mut raw))
            .await
            .ok()?
            .ok()?;
        let header = Header::decode(&raw).ok()?;
        let mut body = vec![0u8; header.length as usize];
        if header.length > 0 {
            tokio::time::timeout(budget, self.stream.read_exact(&mut body))
                .await
                .ok()?
                .ok()?;
        }
        if header.session != 0 {
            self.session = header.session;
        }
        trace.record("<<<", &header, &body);
        Some((header, body))
    }
}

fn trim_json(body: &[u8]) -> Value {
    let mut end = body.len();
    while end > 0 && matches!(body[end - 1], 0 | b'\n' | b'\r') {
        end -= 1;
    }
    serde_json::from_slice(&body[..end]).unwrap_or(Value::Null)
}

fn ret_of(reply: &Value) -> Option<i64> {
    reply.get("Ret").and_then(Value::as_i64)
}

fn session_of(reply: &Value) -> Option<u32> {
    let raw = reply.get("SessionID")?.as_str()?.trim().to_string();
    let hex = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(&raw);
    u32::from_str_radix(hex, 16).ok()
}

fn ffprobe_frames(path: &Path) -> (Option<u64>, Option<String>) {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_frames",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "default=nokey=1:noprint_wrappers=1",
        ])
        .arg(path)
        .output();
    let out = match out {
        Ok(out) => out,
        Err(error) => return (None, Some(error.to_string())),
    };
    if out.status.success() {
        let frames = String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|line| line.trim().parse::<u64>().ok());
        return (frames, None);
    }
    let line = String::from_utf8_lossy(&out.stderr)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let reason = if line.is_empty() {
        format!("ffprobe exit {}", out.status.code().unwrap_or(-1))
    } else {
        line
    };
    (None, Some(reason))
}

#[derive(Clone, Copy)]
struct Deltas {
    token_login: bool,
    twice_login: bool,
    claim_wait: bool,
    random: bool,
    system_info: bool,
}

impl Deltas {
    fn of(mode: Mode) -> Self {
        match mode {
            Mode::Plaintext | Mode::Sealed => Self {
                token_login: false,
                twice_login: false,
                claim_wait: true,
                random: true,
                system_info: true,
            },
            Mode::Replica => Self {
                token_login: true,
                twice_login: true,
                claim_wait: false,
                random: true,
                system_info: false,
            },
        }
    }

    fn apply(&mut self, raw: &str) {
        for token in raw.split(',') {
            match token.trim().to_ascii_lowercase().as_str() {
                "token" => self.token_login = true,
                "password" => self.token_login = false,
                "twice" => self.twice_login = true,
                "once" => self.twice_login = false,
                "wait" => self.claim_wait = true,
                "nowait" => self.claim_wait = false,
                "random" => self.random = true,
                "norandom" => self.random = false,
                "sysinfo" => self.system_info = true,
                "nosysinfo" => self.system_info = false,
                _ => {}
            }
        }
    }

    fn words(&self) -> String {
        let login = if self.token_login {
            "token"
        } else {
            "password"
        };
        let twice = if self.twice_login { "twice" } else { "once" };
        let wait = if self.claim_wait { "wait" } else { "nowait" };
        let random = if self.random { "random" } else { "norandom" };
        let sysinfo = if self.system_info {
            "sysinfo"
        } else {
            "nosysinfo"
        };
        format!("{login},{twice},{wait},{random},{sysinfo}")
    }
}

fn claim_body(session: u32, stream: &str, random: Option<String>) -> Value {
    let mut body = json!({
        "Name": "OPMonitor",
        "OPMonitor": {
            "Action": "Claim",
            "Parameter": {
                "Channel": 0,
                "CombinMode": "CONNECT_ALL",
                "StreamType": stream,
                "TransMode": "TCP",
            },
        },
        "SessionID": format!("0x{session:08x}"),
    });
    if let Some(value) = random {
        body["DHParameter"] = json!({ "RandomStrA": value });
    }
    body
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    load_secrets();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode_raw = {
        let from_args = args.first().cloned().unwrap_or_default();
        if from_args.is_empty() {
            let from_env = env("PROVIEW_PROBE_MODE");
            if from_env.is_empty() {
                "plaintext".to_string()
            } else {
                from_env
            }
        } else {
            from_args
        }
    };
    let mode = Mode::parse(&mode_raw).ok_or("mode must be plaintext, sealed or replica")?;
    let seconds: u64 = args
        .get(1)
        .and_then(|value| value.parse().ok())
        .or_else(|| env("PROVIEW_PROBE_SECONDS").parse().ok())
        .unwrap_or(9);
    let stream_kind = {
        let raw = env("PROVIEW_STREAM");
        if raw.is_empty() {
            "Main".to_string()
        } else {
            raw
        }
    };

    let account = env("PROVIEW_EMAIL");
    let password = env("PROVIEW_PASSWORD");
    let serial = env("PROVIEW_DEVICE_SERIAL");
    let device_user = env("PROVIEW_DEVICE_USER");
    let device_pass = env("PROVIEW_DEVICE_PASS");
    if account.is_empty() || password.is_empty() || serial.is_empty() {
        return Err("set PROVIEW_EMAIL, PROVIEW_PASSWORD and PROVIEW_DEVICE_SERIAL".into());
    }
    let identity = Identity::resolve();
    if !identity.is_complete() {
        return Err(proview_core::cloud::MISSING_CLOUD_IDENTITY.into());
    }

    let run_id = format!("{}-{}", stamp(SystemTime::now()), mode.as_str());
    let out_dir = oracle_dir();
    fs::create_dir_all(&out_dir)?;
    set_private_dir(&out_dir);
    let log_path = out_dir.join(format!("{run_id}.log"));
    let json_path = out_dir.join(format!("{run_id}.json"));
    let es_path = out_dir.join(format!("{run_id}.es"));

    let mut trace = Trace::new();
    let mut timings: BTreeMap<&'static str, f64> = BTreeMap::new();
    let mut notes: Vec<String> = Vec::new();

    let started = Instant::now();
    let cloud = EnvelopeClient::new(identity);
    let tokens = cloud.login_envelope(&account, &password).await?;
    timings.insert("cloud_login_ms", started.elapsed().as_secs_f64() * 1000.0);

    let started = Instant::now();
    let devices = cloud.device_list(&tokens.access).await?;
    timings.insert("device_list_ms", started.elapsed().as_secs_f64() * 1000.0);
    let device = devices
        .iter()
        .find(|entry| entry.uuid == serial)
        .ok_or("the serial is not on this account")?;
    let rps_id = device
        .extra
        .get("rps")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let device_token = device
        .extra
        .get("deviceToken")
        .and_then(|value| value.get("AdminToken"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if rps_id.is_empty() {
        return Err("the device list carries no rps id".into());
    }
    notes.push(format!(
        "device token present={} length={}",
        !device_token.is_empty(),
        device_token.len()
    ));

    let started = Instant::now();
    let status = StatusClient::new("pub-status.secu100.net")
        .query(&serial, &auth_code_of(device))
        .await?;
    timings.insert("status_ms", started.elapsed().as_secs_f64() * 1000.0);
    let (host, port) = status
        .access_server()
        .ok_or("no status cluster reported an access server")?;

    let session_id = rps::session_id();
    let client_token = rps::client_token();
    let request = p2p::agent_request(
        &rps_id,
        &client_token,
        &serial,
        p2p::AGENT_SERVICE_MEDIA,
        &session_id,
        p2p::AGENT_STEP_MEDIA,
    );
    let started = Instant::now();
    let agent = rps::AccessClient::new()
        .assign(&host, port, &request)
        .await?;
    timings.insert("rps_assign_ms", started.elapsed().as_secs_f64() * 1000.0);

    let frame = p2p::handshake_frame(&rps_id, &client_token, &serial, &session_id);
    let started = Instant::now();
    let tunnel = rps::open(&agent, &frame).await?;
    timings.insert("handshake_ms", started.elapsed().as_secs_f64() * 1000.0);
    let dest = tunnel.reply.dest_ip.clone();
    trace.close(&format!(
        "relay accepted={} error={} dest={} mtu={}",
        tunnel.reply.accepted(),
        tunnel.reply.error_num,
        dest,
        tunnel.reply.mtu
    ));
    if !tunnel.reply.accepted() {
        return Err(format!("relay refused the session: {}", tunnel.reply.error_num).into());
    }
    let mut wire = Wire::new(tunnel.stream);

    let mut deltas = Deltas::of(mode);
    deltas.apply(&env("PROVIEW_PROBE_DELTAS"));
    let token_login = deltas.token_login && !device_token.is_empty();

    let started = Instant::now();
    let login_seconds = if token_login { "token" } else { "password" };
    let login_value = if token_login {
        json!({
            "EncryptType": "TOKEN",
            "LoginType": "DVRIP-FutureHome",
            "UserName": device_token,
            "PassWord": "",
            "CommunicateKey": "",
        })
    } else {
        json!({
            "EncryptType": "MD5",
            "LoginType": "DVRIP-Web",
            "PassWord": crypto::sofia_hash(&device_pass),
            "UserName": device_user,
        })
    };
    let (login_channel, login_end) = if token_login {
        (MONITOR_CHANNEL, false)
    } else {
        (0, true)
    };
    let logins = if deltas.twice_login { 2 } else { 1 };
    let mut login_ret: Option<i64> = None;
    let mut login_rets: Vec<Option<i64>> = Vec::new();
    for _ in 0..logins {
        wire.send_json(
            &mut trace,
            login_channel,
            login_end,
            LOGIN_ID,
            &login_value,
            !token_login,
        )
        .await?;
        let reply = wire
            .recv(&mut trace, Duration::from_secs(6))
            .await
            .ok_or("no login reply")?
            .1;
        let reply_json = trim_json(&reply);
        login_ret = ret_of(&reply_json);
        login_rets.push(login_ret);
        if let Some(session) = session_of(&reply_json) {
            wire.session = session;
        }
    }
    timings.insert("dvrip_login_ms", started.elapsed().as_secs_f64() * 1000.0);
    notes.push(format!(
        "dvrip login mode={login_seconds} frames={logins} rets={login_rets:?} session=0x{:08x}",
        wire.session
    ));

    let random = if deltas.random {
        Some(rps::random_lower(4))
    } else {
        None
    };
    let claim = claim_body(wire.session, &stream_kind, random);
    let started = Instant::now();
    wire.send_json(
        &mut trace,
        MONITOR_CHANNEL,
        false,
        MONITOR_CLAIM,
        &claim,
        false,
    )
    .await?;
    let mut claim_ret: Option<i64> = None;
    if !deltas.claim_wait {
        notes.push("claim reply not awaited, as stream() did at 5677181".to_string());
    } else {
        let reply = wire
            .recv(&mut trace, Duration::from_secs(6))
            .await
            .ok_or("no monitor claim reply")?;
        claim_ret = ret_of(&trim_json(&reply.1));
        notes.push(format!(
            "claim reply id=0x{:04x} ret={claim_ret:?}",
            reply.0.id
        ));
        timings.insert("claim_reply_ms", started.elapsed().as_secs_f64() * 1000.0);
    }

    let vekey = match mode {
        Mode::Sealed => random_hex("PROVIEW_VEKEY1", 32),
        _ => String::new(),
    };
    let mut start = start_body(wire.session, 0, &stream_kind);
    if mode == Mode::Sealed {
        start["OPMonitor"]["Parameter"]["VEKEY1"] = Value::String(vekey.clone());
    }
    let started = Instant::now();
    wire.send_json(&mut trace, 0, false, MONITOR_START, &start, false)
        .await?;
    timings.insert("start_sent_ms", started.elapsed().as_secs_f64() * 1000.0);

    if deltas.system_info {
        let system_info = json!({
            "Name": "SystemInfo",
            "SessionID": format!("0x{:08x}", wire.session),
        });
        wire.send_json(&mut trace, 0, true, SYSTEM_INFO_ID, &system_info, true)
            .await?;
    }

    let mut demux = LiveDemux::new();
    let mut elementary = Vec::new();
    let mut media_bodies = 0u64;
    let mut media_bytes = 0u64;
    let mut other_ids: BTreeMap<u16, u64> = BTreeMap::new();
    let mut first_media_ms: Option<f64> = None;
    let mut system_info_ret: Option<i64> = None;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        let budget = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(3));
        if budget.is_zero() {
            break;
        }
        let Some((header, body)) = wire.recv(&mut trace, budget).await else {
            break;
        };
        if header.id == LIVE_MEDIA_ID {
            if first_media_ms.is_none() {
                first_media_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
            }
            media_bodies += 1;
            media_bytes += body.len() as u64;
            elementary.extend_from_slice(&demux.push(&body));
        } else if header.id == SYSTEM_INFO_ID || header.id == SYSTEM_INFO_REPLY {
            system_info_ret = ret_of(&trim_json(&body));
        } else {
            *other_ids.entry(header.id).or_insert(0) += 1;
        }
    }
    elementary.extend_from_slice(&demux.finish());
    if let Some(at) = first_media_ms {
        timings.insert("first_media_ms", at);
    }
    let stats = demux.stats();
    let (ffprobe, ffprobe_error) = if elementary.is_empty() {
        (None, None)
    } else {
        fs::write(&es_path, &elementary)?;
        restrict_file(&es_path);
        ffprobe_frames(&es_path)
    };

    let summary = json!({
        "run_id": run_id,
        "mode": mode.as_str(),
        "stream": stream_kind,
        "seconds": seconds,
        "serial": serial,
        "access_server": format!("{host}:{port}"),
        "relay_dest": dest,
        "login": {
            "mode": login_seconds,
            "frames": logins,
            "rets": login_rets,
            "ret": login_ret,
            "session": format!("0x{:08x}", wire.session),
        },
        "claim_ret": claim_ret,
        "system_info_ret": system_info_ret,
        "vekey1_present": mode == Mode::Sealed,
        "deltas": deltas.words(),
        "vekey1": vekey,
        "timings_ms": timings,
        "media": {
            "bodies": media_bodies,
            "bytes": media_bytes,
            "keyframes": stats.keyframes,
            "deltas": stats.deltas,
            "markers": stats.markers,
            "padding": stats.padding,
            "continuations": stats.continuations,
            "dropped": stats.dropped,
            "elementary_bytes": stats.elementary_bytes,
            "records": stats.keyframes + stats.deltas,
            "decodable_frames": ffprobe,
            "ffprobe_error": ffprobe_error,
        },
        "other_ids": other_ids.iter().map(|(id, count)| json!({
            "id": id, "id_hex": format!("0x{id:04x}"), "count": count
        })).collect::<Vec<_>>(),
        "tally": trace.tally.iter().map(|(id, (count, bytes))| json!({
            "id": id, "id_hex": format!("0x{id:04x}"), "count": count, "bytes": bytes
        })).collect::<Vec<_>>(),
        "control": trace.control,
        "notes": notes,
    });

    trace.close(&format!(
        "media bodies={media_bodies} bytes={media_bytes} keyframes={} deltas={} records={} elementary={} decodable={ffprobe:?}",
        stats.keyframes,
        stats.deltas,
        stats.keyframes + stats.deltas,
        stats.elementary_bytes
    ));
    write_private(&log_path, &format!("{}\n", trace.lines.join("\n")))?;
    write_private(
        &json_path,
        &format!("{}\n", serde_json::to_string_pretty(&summary)?),
    )?;

    println!("run      {run_id}");
    println!(
        "mode     {} ({} s, stream {}, vekey={})",
        mode.as_str(),
        seconds,
        summary["stream"],
        summary["vekey1_present"]
    );
    println!("relay    dest={dest} accepted=true");
    println!(
        "dvrip    login={login_seconds} ret={login_ret:?} claim_ret={claim_ret:?} system_info_ret={system_info_ret:?}"
    );
    println!(
        "media    bodies={media_bodies} bytes={media_bytes} keyframes={} deltas={} records={} elementary={} decodable={:?} ffprobe_error={:?}",
        stats.keyframes,
        stats.deltas,
        stats.keyframes + stats.deltas,
        stats.elementary_bytes,
        ffprobe,
        ffprobe_error
    );
    println!("deltas   {}", deltas.words());
    println!(
        "first_media_ms={} messages={}",
        first_media_ms.map(|value| value.round()).unwrap_or(0.0),
        trace.lines.len()
    );
    println!("log      {}", log_path.display());
    println!("summary  {}", json_path.display());
    if !elementary.is_empty() {
        println!("es       {}", es_path.display());
    }
    for note in &notes {
        println!("note     {note}");
    }
    Ok(())
}
