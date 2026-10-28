use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use proview_core::cloud::envelope::{EnvelopeClient, Identity};
use proview_core::cloud::status::{self, DeviceOnline, StatusClient, STATUS_HOST};
use proview_core::cloud::CloudDevice;
use proview_core::config::{Config, ProviderKind};
use proview_core::dvrip::client::Connection;
use proview_core::provider::p2p::AgentServer;
use proview_core::provider::rps::Tunnel;
use proview_core::provider::{
    p2p, rps, Provider, ProviderError, ProviderSource, Router, StreamTicket, VendorSession,
};
use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::serde::Serialize;
use rocket::State;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;

use crate::config::ModeStore;
use crate::live_state::LiveState;

pub const STAGES: [&str; 8] = [
    "cloud_login",
    "device_list",
    "status_cluster",
    "rps_access",
    "tunnel",
    "dvrip_login",
    "opmonitor_claim",
    "first_frame",
];

const WALK_DEADLINE: Duration = Duration::from_secs(8);
const WALK_COOLDOWN: Duration = Duration::from_millis(5_000);

#[derive(Debug, Clone, Serialize)]
pub struct StageReport {
    pub stage: &'static str,
    pub ok: bool,
    pub elapsed_ms: u64,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Attempt {
    pub device: String,
    pub channel: u32,
    pub at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<&'static str>,
    pub detail: String,
    pub elapsed_ms: u64,
    pub stages: Vec<StageReport>,
}

impl Attempt {
    pub fn failed_stage(reports: &[StageReport]) -> Option<&'static str> {
        reports
            .iter()
            .rev()
            .find(|report| !report.ok)
            .map(|report| report.stage)
    }

    fn from_reports(
        device: &str,
        channel: u32,
        at: u64,
        reports: Vec<StageReport>,
        elapsed_ms: u64,
    ) -> Self {
        let stage = Self::failed_stage(&reports);
        let detail = stage
            .and_then(|stage| {
                reports
                    .iter()
                    .find(|report| report.stage == stage && !report.ok)
                    .map(|report| report.detail.clone())
            })
            .unwrap_or_else(|| {
                "every stage the cloud path makes today answered; the provider stopped after the device login"
                    .to_string()
            });
        Self {
            device: device.to_string(),
            channel,
            at,
            stage,
            detail,
            elapsed_ms,
            stages: reports,
        }
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub device: String,
    pub channel: u32,
}

impl Key {
    pub fn new(device: &str, channel: u32) -> Self {
        Self {
            device: device.to_string(),
            channel,
        }
    }
}

struct Held {
    session: String,
    attempt: Attempt,
}

#[derive(Default)]
pub struct Ledger {
    entries: Mutex<HashMap<Key, Held>>,
    gates: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn gate(&self, device: &str) -> Arc<AsyncMutex<()>> {
        let mut gates = match self.gates.lock() {
            Ok(gates) => gates,
            Err(poisoned) => poisoned.into_inner(),
        };
        gates
            .entry(device.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    pub fn record(&self, session: &str, attempt: Attempt) {
        let key = Key::new(&attempt.device, attempt.channel);
        let mut entries = match self.entries.lock() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        };
        entries.insert(
            key.clone(),
            Held {
                session: session.to_string(),
                attempt,
            },
        );
        let live: std::collections::HashSet<String> =
            entries.keys().map(|key| key.device.clone()).collect();
        drop(entries);
        if let Ok(mut gates) = self.gates.lock() {
            gates.retain(|device, gate| Arc::strong_count(gate) > 1 || live.contains(device));
        }
    }

    pub fn forget(&self, device: &str, channel: u32) {
        let key = Key::new(device, channel);
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(&key);
        }
    }

    pub fn last(&self, session: &str, device: &str, channel: u32) -> Option<Attempt> {
        let entries = self.entries.lock().ok()?;
        let held = entries.get(&Key::new(device, channel))?;
        if held.session != session {
            return None;
        }
        Some(held.attempt.clone())
    }

    pub fn fresh(
        &self,
        session: &str,
        device: &str,
        channel: u32,
        window: Duration,
    ) -> Option<Attempt> {
        let now = unix_ms();
        let window_ms = window.as_millis() as u64;
        let entries = self.entries.lock().ok()?;
        entries
            .iter()
            .filter(|(key, held)| {
                key.device == device
                    && held.session == session
                    && now.saturating_sub(held.attempt.at) < window_ms
            })
            .max_by_key(|(_, held)| held.attempt.at)
            .map(|(_, held)| {
                let mut attempt = held.attempt.clone();
                attempt.channel = channel;
                attempt
            })
    }

    #[cfg(test)]
    fn gate_count(&self) -> usize {
        self.gates.lock().map(|gates| gates.len()).unwrap_or(0)
    }

    pub fn attempts(&self, session: &str) -> Vec<Attempt> {
        let mut attempts: Vec<Attempt> = self
            .entries
            .lock()
            .map(|entries| {
                entries
                    .values()
                    .filter(|held| held.session == session)
                    .map(|held| held.attempt.clone())
                    .collect()
            })
            .unwrap_or_default();
        attempts.sort_by_key(|attempt| std::cmp::Reverse(attempt.at));
        attempts
    }
}

static LEDGER: LazyLock<Ledger> = LazyLock::new(Ledger::new);

pub fn ledger() -> &'static Ledger {
    &LEDGER
}

struct Wire {
    client: String,
    session: String,
}

struct Entry {
    device: CloudDevice,
    rps: String,
    device_token: String,
}

struct Walk<'a> {
    cfg: &'a Config,
    session: &'a VendorSession,
    device: &'a str,
    channel: u32,
    high: bool,
    status_host: String,
    started: Instant,
}

impl<'a> Walk<'a> {
    fn new(
        cfg: &'a Config,
        session: &'a VendorSession,
        device: &'a str,
        channel: u32,
        high: bool,
    ) -> Self {
        Self {
            cfg,
            session,
            device,
            channel,
            high,
            status_host: STATUS_HOST.to_string(),
            started: Instant::now(),
        }
    }

    async fn bounded<T>(
        &self,
        reports: &mut Vec<StageReport>,
        stage: &'static str,
        started: Instant,
        future: impl Future<Output = Result<(T, String), String>>,
    ) -> Result<(T, String), String> {
        let spent = self.started.elapsed();
        let left = WALK_DEADLINE.saturating_sub(spent);
        let outcome = if left.is_zero() {
            Err(format!(
                "the walk spent its {} ms budget before this stage",
                WALK_DEADLINE.as_millis()
            ))
        } else {
            match timeout(left, future).await {
                Ok(outcome) => outcome,
                Err(_) => Err(format!("no answer within {} ms", left.as_millis())),
            }
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let (ok, detail) = match &outcome {
            Ok((_, detail)) => (true, detail.clone()),
            Err(detail) => (false, detail.clone()),
        };
        reports.push(StageReport {
            stage,
            ok,
            elapsed_ms,
            detail,
        });
        outcome
    }

    async fn run(mut self) -> Attempt {
        let reports = self.stages().await;
        Attempt::from_reports(
            self.device,
            self.channel,
            unix_ms(),
            reports,
            self.started.elapsed().as_millis() as u64,
        )
    }

    async fn stages(&mut self) -> Vec<StageReport> {
        let mut reports: Vec<StageReport> = Vec::with_capacity(STAGES.len());

        let started = Instant::now();
        let token = match self.session.token.as_deref() {
            Some(token) if !token.trim().is_empty() => {
                reports.push(StageReport {
                    stage: "cloud_login",
                    ok: true,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    detail: "the session carries a cloud token".to_string(),
                });
                token.to_string()
            }
            _ => {
                reports.push(StageReport {
                    stage: "cloud_login",
                    ok: false,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    detail: "the session carries no cloud token; sign in again".to_string(),
                });
                return reports;
            }
        };

        let started = Instant::now();
        let entry = match self
            .bounded(&mut reports, "device_list", started, self.list(&token))
            .await
        {
            Ok((entry, _)) => entry,
            Err(_) => return reports,
        };

        let started = Instant::now();
        let state = match self
            .bounded(
                &mut reports,
                "status_cluster",
                started,
                self.cluster(&entry),
            )
            .await
        {
            Ok((state, _)) => state,
            Err(_) => return reports,
        };

        let wire = Wire {
            client: rps::client_token(),
            session: rps::session_id(),
        };

        let started = Instant::now();
        let agent = match self
            .bounded(
                &mut reports,
                "rps_access",
                started,
                self.assign(&entry, &state, &wire),
            )
            .await
        {
            Ok((agent, _)) => agent,
            Err(_) => return reports,
        };

        let started = Instant::now();
        let tunnel = match self
            .bounded(
                &mut reports,
                "tunnel",
                started,
                self.open(&entry, &agent, &wire),
            )
            .await
        {
            Ok((tunnel, _)) => tunnel,
            Err(_) => return reports,
        };

        let started = Instant::now();
        self.bounded(
            &mut reports,
            "dvrip_login",
            started,
            self.login_device(&entry, tunnel),
        )
        .await
        .ok();

        reports
    }

    async fn list(&self, token: &str) -> Result<(Entry, String), String> {
        let client = EnvelopeClient::with_base(Identity::resolve(), self.cfg.cloud_base.clone());
        let devices = client
            .device_list(token)
            .await
            .map_err(|error| error.to_string())?;
        let found = devices
            .iter()
            .find(|entry| entry.uuid == self.device.trim())
            .ok_or_else(|| {
                format!(
                    "none of the {} devices listed carries the id {}",
                    devices.len(),
                    self.device.trim()
                )
            })?;
        let rps = found
            .extra
            .get("rps")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if rps.is_empty() {
            return Err(
                "the device list carries no rps id, so the relay cannot be asked".to_string(),
            );
        }
        let device_token = found
            .extra
            .get("deviceToken")
            .and_then(|value| value.get("AdminToken"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let detail = format!(
            "{} devices listed, {} carries rps {}",
            devices.len(),
            found.uuid,
            if rps.is_empty() { "no" } else { rps.as_str() }
        );
        Ok((
            Entry {
                device: found.clone(),
                rps,
                device_token,
            },
            detail,
        ))
    }

    async fn cluster(&self, entry: &Entry) -> Result<(DeviceOnline, String), String> {
        let client = StatusClient::new(self.status_host.clone());
        let state = client
            .query(&entry.device.uuid, &status::auth_code_of(&entry.device))
            .await
            .map_err(|error| error.to_string())?;
        let detail = state.summary();
        Ok((state, detail))
    }

    async fn assign(
        &self,
        entry: &Entry,
        state: &DeviceOnline,
        wire: &Wire,
    ) -> Result<(AgentServer, String), String> {
        let (host, port) = state
            .access_server()
            .ok_or_else(|| format!("{} (no cluster reported an access server)", state.summary()))?;
        let request = p2p::agent_request(
            &entry.rps,
            &wire.client,
            &entry.device.uuid,
            p2p::AGENT_SERVICE_MEDIA,
            &wire.session,
            p2p::AGENT_STEP_MEDIA,
        );
        let agent = rps::AccessClient::new()
            .assign(&host, port, &request)
            .await
            .map_err(|error| format!("the access server {host}:{port} refused: {error}"))?;
        let detail = format!(
            "the access server {host}:{port} assigned {}:{}",
            agent.host, agent.port
        );
        Ok((agent, detail))
    }

    async fn open(
        &self,
        entry: &Entry,
        agent: &AgentServer,
        wire: &Wire,
    ) -> Result<(Tunnel, String), String> {
        let frame =
            p2p::handshake_frame(&entry.rps, &wire.client, &entry.device.uuid, &wire.session);
        let tunnel = rps::open(agent, &frame)
            .await
            .map_err(|error| format!("the relay handshake failed: {error}"))?;
        if !tunnel.reply.accepted() {
            return Err(format!(
                "the relay refused the session: ErrorNum {}",
                tunnel.reply.error_num
            ));
        }
        let detail = format!(
            "the relay accepted the session for {} at mtu {}",
            tunnel.reply.dest_ip, tunnel.reply.mtu
        );
        Ok((tunnel, detail))
    }

    async fn login_device(
        &self,
        entry: &Entry,
        tunnel: Tunnel,
    ) -> Result<(Connection, String), String> {
        let mut connection = tunnel.into_connection();
        let detail = if entry.device_token.is_empty() {
            let login = connection
                .login(&self.cfg.device_user, &self.cfg.device_pass)
                .await
                .map_err(|error| format!("the device login was refused: {error}"))?;
            format!("password login answered with session {}", login.session)
        } else {
            let reply = connection
                .login_token(&entry.device_token, "")
                .await
                .map_err(|error| format!("the token login was refused: {error}"))?;
            format!(
                "token login answered with ret {}",
                reply
                    .get("Ret")
                    .map(|value| value.to_string())
                    .unwrap_or_default()
            )
        };
        let _ = self.high;
        Ok((connection, detail))
    }
}

fn worth_walking(error: &ProviderError) -> bool {
    !matches!(
        error,
        ProviderError::BadCredentials
            | ProviderError::Expired
            | ProviderError::UnknownDevice(_)
            | ProviderError::NotImplemented(_)
            | ProviderError::Misconfigured(_)
    )
}

async fn diagnose(
    cfg: &Config,
    session: &str,
    vendor: &VendorSession,
    device: &str,
    channel: u32,
    high: bool,
) -> Attempt {
    let gate = ledger().gate(device);
    let _held = gate.lock().await;
    if let Some(previous) = ledger().fresh(session, device, channel, WALK_COOLDOWN) {
        ledger().record(session, previous.clone());
        return previous;
    }
    let attempt = Walk::new(cfg, vendor, device, channel, high).run().await;
    ledger().record(session, attempt.clone());
    attempt
}

#[derive(Serialize)]
pub struct FailureBody {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub stages: Vec<StageReport>,
}

pub(crate) type Failure = (Status, Json<Box<FailureBody>>);

fn widen((status, Json(body)): crate::routes::Failure) -> Failure {
    (
        status,
        Json(Box::new(FailureBody {
            ok: false,
            message: body.message,
            code: body.code,
            stage: None,
            detail: None,
            elapsed_ms: None,
            stages: Vec::new(),
        })),
    )
}

#[get("/stream?<device>&<channel>&<quality>")]
pub async fn stream(
    device: String,
    channel: Option<u32>,
    quality: Option<String>,
    cfg: &State<Config>,
    store: &State<Store>,
    modes: &State<ModeStore>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StreamTicket>, Failure> {
    let id = crate::routes::session_id(cookies).map_err(widen)?;
    let vendor = crate::routes::restore(store, &id).map_err(widen)?;
    let router = Router::new(cfg, modes.mode_for(Some(&id)));
    let channel = channel.unwrap_or(1);
    let high = quality.as_deref() != Some("sub");
    let error = match router.stream(&vendor, &device, channel, high).await {
        Ok(ticket) => {
            ledger().forget(&device, channel);
            return Ok(Json(ticket));
        }
        Err(error) => error,
    };
    let status = crate::routes::status_for(&error);
    let attempt = if cfg.provider == ProviderKind::Proview
        && router.source() == ProviderSource::Cloud
        && worth_walking(&error)
    {
        Some(diagnose(cfg, &id, &vendor, &device, channel, high).await)
    } else {
        None
    };
    let mut body = FailureBody {
        ok: false,
        message: error.to_string(),
        code: None,
        stage: None,
        detail: None,
        elapsed_ms: None,
        stages: Vec::new(),
    };
    if let Some(attempt) = attempt {
        body.stage = attempt.stage;
        body.detail = attempt.stage.map(|_| attempt.detail.clone());
        body.elapsed_ms = Some(attempt.elapsed_ms);
        body.stages = attempt.stages;
    }
    Err((status, Json(Box::new(body))))
}

#[derive(Serialize)]
pub struct LiveReport {
    pub active_streams: usize,
    pub streams: Vec<crate::stream::StreamStats>,
    pub frames: u64,
    pub bytes: u64,
    pub gaps: u64,
    pub attempts: Vec<Attempt>,
}

#[get("/live/stats")]
pub fn stats(
    cookies: &CookieJar<'_>,
    store: &State<Store>,
    live: &State<LiveState>,
) -> Result<Json<LiveReport>, crate::routes::Failure> {
    let id = crate::routes::session_id(cookies)?;
    crate::routes::restore(store, &id)?;
    let snapshot = crate::stream::snapshot(live);
    let frames = snapshot.streams.iter().map(|stream| stream.packets).sum();
    let bytes = snapshot.streams.iter().map(|stream| stream.bytes).sum();
    let gaps = snapshot.streams.iter().map(|stream| stream.drops).sum();
    Ok(Json(LiveReport {
        active_streams: snapshot.active_streams,
        streams: snapshot.streams,
        frames,
        bytes,
        gaps,
        attempts: ledger().attempts(&id),
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    fn report(stage: &'static str, ok: bool) -> StageReport {
        StageReport {
            stage,
            ok,
            elapsed_ms: 12,
            detail: format!("{stage} detail"),
        }
    }

    #[test]
    fn a_stage_report_serialises_to_the_contract_shape() {
        let json = serde_json::to_value(report("tunnel", false)).expect("report json");
        assert_eq!(
            json,
            serde_json::json!({
                "stage": "tunnel",
                "ok": false,
                "elapsed_ms": 12,
                "detail": "tunnel detail",
            })
        );
    }

    #[test]
    fn only_a_report_that_failed_names_the_stage() {
        assert_eq!(Attempt::failed_stage(&[]), None);
        assert_eq!(
            Attempt::failed_stage(&[report("cloud_login", true), report("device_list", true)]),
            None
        );
        assert_eq!(
            Attempt::failed_stage(&[
                report("cloud_login", true),
                report("device_list", true),
                report("status_cluster", false),
            ]),
            Some("status_cluster")
        );
        let walked = STAGES
            .iter()
            .take(6)
            .map(|stage| report(stage, true))
            .collect::<Vec<_>>();
        assert_eq!(Attempt::failed_stage(&walked), None);
        let attempt = Attempt::from_reports("cam", 1, unix_ms(), walked, 9);
        assert_eq!(attempt.stage, None);
        let json = serde_json::to_value(&attempt).expect("attempt json");
        assert!(json.get("stage").is_none(), "{json}");
    }

    #[test]
    fn only_the_stages_the_walk_runs_are_reported() {
        let stages = &STAGES[..6];
        assert_eq!(stages.last().copied(), Some("dvrip_login"));
        assert!(!stages.contains(&"opmonitor_claim"));
        assert!(!stages.contains(&"first_frame"));
    }

    #[test]
    fn the_gate_map_does_not_grow_with_every_failing_key() {
        let ledger = Ledger::new();
        for index in 0..50 {
            let device = format!("cam-{index}");
            let gate = ledger.gate(&device);
            ledger.record(
                "session-a",
                Attempt {
                    device: device.clone(),
                    channel: 1,
                    at: unix_ms(),
                    stage: Some("device_list"),
                    detail: "refused".to_string(),
                    elapsed_ms: 1,
                    stages: vec![report("device_list", false)],
                },
            );
            ledger.forget(&device, 1);
            drop(gate);
        }
        ledger.record(
            "session-a",
            Attempt {
                device: "last".to_string(),
                channel: 1,
                at: unix_ms(),
                stage: None,
                detail: String::new(),
                elapsed_ms: 0,
                stages: Vec::new(),
            },
        );
        assert!(
            ledger.gate_count() <= 2,
            "the gate map kept {} entries",
            ledger.gate_count()
        );
    }

    #[test]
    fn a_ledger_entry_belongs_to_the_session_that_made_it() {
        let ledger = Ledger::new();
        ledger.record(
            "session-a",
            Attempt {
                device: "cam".to_string(),
                channel: 1,
                at: unix_ms(),
                stage: Some("tunnel"),
                detail: "the relay refused the session: ErrorNum 403".to_string(),
                elapsed_ms: 41,
                stages: vec![report("cloud_login", true), report("tunnel", false)],
            },
        );
        assert_eq!(
            ledger
                .last("session-a", "cam", 1)
                .map(|attempt| attempt.stage),
            Some(Some("tunnel"))
        );
        assert!(ledger.last("session-b", "cam", 1).is_none());
        assert!(ledger
            .fresh("session-a", "cam", 1, Duration::from_secs(30))
            .is_some());
        assert!(ledger
            .fresh("session-a", "cam", 1, Duration::ZERO)
            .is_none());
        assert!(ledger
            .fresh("session-b", "cam", 1, Duration::from_secs(30))
            .is_none());
        assert!(ledger.last("session-a", "cam", 4).is_none());
        let reused = ledger
            .fresh("session-a", "cam", 4, Duration::from_secs(30))
            .expect("the same device answers for another channel");
        assert_eq!(reused.channel, 4);
        assert_eq!(reused.stage, Some("tunnel"));
        assert!(ledger
            .fresh("session-a", "other-cam", 1, Duration::from_secs(30))
            .is_none());
        assert_eq!(ledger.attempts("session-a").len(), 1);
        assert!(ledger.attempts("session-b").is_empty());
        ledger.forget("cam", 1);
        assert!(ledger.attempts("session-a").is_empty());
    }

    async fn stub_cloud(requests: Arc<AtomicUsize>, devices: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("stub listener");
        let address = listener.local_addr().expect("stub address");
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let requests = requests.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 4096];
                    let _ = socket.read(&mut buffer).await;
                    requests.fetch_add(1, AtomicOrdering::SeqCst);
                    let body = format!("{{\"code\":200,\"data\":{{\"mine\":{devices}}}}}");
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn four_parallel_failures_for_one_channel_share_a_single_walk() {
        let requests = Arc::new(AtomicUsize::new(0));
        let base = stub_cloud(requests.clone(), "[]").await;
        let cfg = Config {
            provider: ProviderKind::Proview,
            cloud_base: base,
            device_user: "admin".to_string(),
            device_pass: "secret".to_string(),
            fixture_dir: "fixtures".to_string(),
            device_host: None,
            device_rtsp: None,
            device_onvif: None,
            session_ttl_secs: 3600,
        };
        let session = VendorSession {
            account: "parallel@example.invalid".to_string(),
            token: Some("token".to_string()),
        };
        let mut handles = Vec::new();
        for _ in 0..4 {
            let cfg = cfg.clone();
            let session = session.clone();
            handles.push(tokio::spawn(async move {
                diagnose(&cfg, "parallel-session", &session, "parallel-cam", 4, true).await
            }));
        }
        let mut stages = Vec::new();
        for handle in handles {
            let attempt = handle.await.expect("walk task");
            assert_eq!(attempt.device, "parallel-cam");
            assert_eq!(attempt.channel, 4);
            stages.push(attempt.stage);
        }
        assert_eq!(requests.load(AtomicOrdering::SeqCst), 1);
        assert!(
            stages.iter().all(|stage| *stage == Some("device_list")),
            "{stages:?}"
        );
        ledger().forget("parallel-cam", 4);
    }

    #[tokio::test]
    async fn a_device_without_an_rps_id_fails_at_the_device_list_like_the_provider() {
        let requests = Arc::new(AtomicUsize::new(0));
        let base = stub_cloud(
            requests.clone(),
            r#"[{"uuid":"no-rps-cam","nickname":"No rps","channelNum":1}]"#,
        )
        .await;
        let cfg = Config {
            provider: ProviderKind::Proview,
            cloud_base: base,
            device_user: "admin".to_string(),
            device_pass: "secret".to_string(),
            fixture_dir: "fixtures".to_string(),
            device_host: None,
            device_rtsp: None,
            device_onvif: None,
            session_ttl_secs: 3600,
        };
        let session = VendorSession {
            account: "rps@example.invalid".to_string(),
            token: Some("token".to_string()),
        };
        let attempt = Walk::new(&cfg, &session, "no-rps-cam", 1, true).run().await;
        assert_eq!(attempt.stage, Some("device_list"));
        assert!(attempt.detail.contains("no rps id"), "{}", attempt.detail);
        assert_eq!(attempt.stages.len(), 2);
        assert_eq!(requests.load(AtomicOrdering::SeqCst), 1);
        assert!(!attempt
            .stages
            .iter()
            .any(|row| row.stage == "status_cluster"));
    }

    #[tokio::test]
    async fn four_parallel_channels_of_one_device_share_a_single_walk() {
        let requests = Arc::new(AtomicUsize::new(0));
        let base = stub_cloud(requests.clone(), "[]").await;
        let cfg = Config {
            provider: ProviderKind::Proview,
            cloud_base: base,
            device_user: "admin".to_string(),
            device_pass: "secret".to_string(),
            fixture_dir: "fixtures".to_string(),
            device_host: None,
            device_rtsp: None,
            device_onvif: None,
            session_ttl_secs: 3600,
        };
        let session = VendorSession {
            account: "grid@example.invalid".to_string(),
            token: Some("token".to_string()),
        };
        let mut handles = Vec::new();
        for channel in 1..=4 {
            let cfg = cfg.clone();
            let session = session.clone();
            handles.push(tokio::spawn(async move {
                diagnose(&cfg, "grid-session", &session, "grid-cam", channel, true).await
            }));
        }
        let mut channels = Vec::new();
        for handle in handles {
            let attempt = handle.await.expect("walk task");
            assert_eq!(attempt.device, "grid-cam");
            assert_eq!(attempt.stage, Some("device_list"));
            channels.push(attempt.channel);
        }
        assert_eq!(requests.load(AtomicOrdering::SeqCst), 1);
        channels.sort_unstable();
        assert_eq!(channels, vec![1, 2, 3, 4]);
        ledger().forget("grid-cam", 1);
        ledger().forget("grid-cam", 2);
        ledger().forget("grid-cam", 3);
        ledger().forget("grid-cam", 4);
    }

    #[tokio::test]
    async fn an_impossible_cloud_base_stops_the_walk_at_the_device_list() {
        let cfg = Config {
            provider: ProviderKind::Proview,
            cloud_base: "http://127.0.0.1:1".to_string(),
            device_user: "admin".to_string(),
            device_pass: "secret".to_string(),
            fixture_dir: "fixtures".to_string(),
            device_host: None,
            device_rtsp: None,
            device_onvif: None,
            session_ttl_secs: 3600,
        };
        let session = VendorSession {
            account: "walk@example.invalid".to_string(),
            token: Some("token".to_string()),
        };
        let attempt = Walk::new(&cfg, &session, "fixture-dev-0001", 1, true)
            .run()
            .await;
        assert_eq!(attempt.stage, Some("device_list"));
        assert_eq!(attempt.stages.len(), 2);
        assert!(attempt.stages[0].ok);
        assert_eq!(attempt.stages[0].stage, "cloud_login");
        assert!(!attempt.stages[1].ok);
        assert!(!attempt.stages[1].detail.is_empty());
    }

    #[tokio::test]
    async fn a_session_without_a_token_stops_the_walk_before_any_call() {
        let cfg = Config {
            provider: ProviderKind::Proview,
            cloud_base: "http://127.0.0.1:1".to_string(),
            device_user: "admin".to_string(),
            device_pass: String::new(),
            fixture_dir: "fixtures".to_string(),
            device_host: None,
            device_rtsp: None,
            device_onvif: None,
            session_ttl_secs: 3600,
        };
        let session = VendorSession {
            account: "walk@example.invalid".to_string(),
            token: None,
        };
        let attempt = Walk::new(&cfg, &session, "fixture-dev-0001", 1, true)
            .run()
            .await;
        assert_eq!(attempt.stage, Some("cloud_login"));
        assert_eq!(attempt.stages.len(), 1);
        assert!(!attempt.stages[0].ok);
    }
}
