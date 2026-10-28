use std::collections::HashMap;
use std::future::Future;
use std::sync::{LazyLock, Mutex};
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

const STAGE_LIMIT: Duration = Duration::from_secs(8);
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
    pub stage: &'static str,
    pub detail: String,
    pub elapsed_ms: u64,
    pub stages: Vec<StageReport>,
}

pub fn first_unproven(stages: &[StageReport]) -> &'static str {
    STAGES
        .iter()
        .copied()
        .find(|stage| {
            !stages
                .iter()
                .any(|report| report.stage == *stage && report.ok)
        })
        .unwrap_or("first_frame")
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Default)]
pub struct Ledger {
    entries: Mutex<HashMap<String, Attempt>>,
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, attempt: Attempt) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(attempt.device.clone(), attempt);
        }
    }

    pub fn forget(&self, device: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(device);
        }
    }

    pub fn last(&self, device: &str) -> Option<Attempt> {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(device).cloned())
    }

    pub fn fresh(&self, device: &str, window: Duration) -> Option<Attempt> {
        self.last(device)
            .filter(|attempt| unix_ms().saturating_sub(attempt.at) < window.as_millis() as u64)
    }

    pub fn attempts(&self) -> Vec<Attempt> {
        let mut attempts: Vec<Attempt> = self
            .entries
            .lock()
            .map(|entries| entries.values().cloned().collect())
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
}

async fn bounded<T>(future: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    match timeout(STAGE_LIMIT, future).await {
        Ok(outcome) => outcome,
        Err(_) => Err(format!("no answer within {} ms", STAGE_LIMIT.as_millis())),
    }
}

fn record<T>(
    reports: &mut Vec<StageReport>,
    stage: &'static str,
    started: Instant,
    outcome: Result<(T, String), String>,
) -> Result<(T, String), String> {
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

impl Walk<'_> {
    async fn run(&self) -> Attempt {
        let started = Instant::now();
        let stages = self.stages().await;
        let stage = first_unproven(&stages);
        let detail = stages
            .iter()
            .find(|report| report.stage == stage && !report.ok)
            .map(|report| report.detail.clone())
            .unwrap_or_else(|| {
                "every stage up to opmonitor_claim answered and no frame was read after it"
                    .to_string()
            });
        Attempt {
            device: self.device.to_string(),
            channel: self.channel,
            at: unix_ms(),
            stage,
            detail,
            elapsed_ms: started.elapsed().as_millis() as u64,
            stages,
        }
    }

    async fn stages(&self) -> Vec<StageReport> {
        let mut reports: Vec<StageReport> = Vec::with_capacity(STAGES.len());

        let mut started = Instant::now();
        let token = match self.session.token.as_deref() {
            Some(token) if !token.trim().is_empty() => {
                record(
                    &mut reports,
                    "cloud_login",
                    started,
                    Ok((
                        (),
                        format!(
                            "the session carries the cloud token for {}",
                            self.session.account
                        ),
                    )),
                )
                .ok();
                token.to_string()
            }
            _ => {
                record::<()>(
                    &mut reports,
                    "cloud_login",
                    started,
                    Err("the session carries no cloud token; sign in again".to_string()),
                )
                .ok();
                return reports;
            }
        };

        started = Instant::now();
        let entry = match record(
            &mut reports,
            "device_list",
            started,
            bounded(self.list(&token)).await,
        ) {
            Ok((entry, _)) => entry,
            Err(_) => return reports,
        };

        started = Instant::now();
        let state = match record(
            &mut reports,
            "status_cluster",
            started,
            bounded(self.cluster(&entry)).await,
        ) {
            Ok((state, _)) => state,
            Err(_) => return reports,
        };

        let wire = Wire {
            client: rps::client_token(),
            session: rps::session_id(),
        };

        started = Instant::now();
        let agent = match record(
            &mut reports,
            "rps_access",
            started,
            bounded(self.assign(&entry, &state, &wire)).await,
        ) {
            Ok((agent, _)) => agent,
            Err(_) => return reports,
        };

        started = Instant::now();
        let tunnel = match record(
            &mut reports,
            "tunnel",
            started,
            bounded(self.open(&entry, &agent, &wire)).await,
        ) {
            Ok((tunnel, _)) => tunnel,
            Err(_) => return reports,
        };

        started = Instant::now();
        let mut connection = match record(
            &mut reports,
            "dvrip_login",
            started,
            bounded(self.login_device(&entry, tunnel)).await,
        ) {
            Ok((connection, _)) => connection,
            Err(_) => return reports,
        };

        started = Instant::now();
        record(
            &mut reports,
            "opmonitor_claim",
            started,
            bounded(self.claim(&mut connection)).await,
        )
        .ok();

        let _ = timeout(Duration::from_millis(400), connection.logout()).await;
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
        if entry.rps.is_empty() {
            return Err(
                "the device list carries no rps id, so no access server can be asked".to_string(),
            );
        }
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
        Ok((connection, detail))
    }

    async fn claim(&self, connection: &mut Connection) -> Result<((), String), String> {
        let channel = self.channel.clamp(1, u32::from(u8::MAX)) as u8 - 1;
        let stream = if self.high { "Main" } else { "Sub" };
        connection
            .claim_monitor(channel, stream)
            .await
            .map_err(|error| format!("the monitor claim was refused: {error}"))?;
        Ok((
            (),
            format!("channel {} claimed on the {stream} stream", self.channel),
        ))
    }
}

fn status_host() -> String {
    std::env::var("PROVIEW_STATUS_HOST")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| STATUS_HOST.to_string())
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
    vendor: &VendorSession,
    device: &str,
    channel: u32,
    high: bool,
) -> Attempt {
    if let Some(previous) = ledger().fresh(device, WALK_COOLDOWN) {
        return previous;
    }
    let attempt = Walk {
        cfg,
        session: vendor,
        device,
        channel,
        high,
        status_host: status_host(),
    }
    .run()
    .await;
    ledger().record(attempt.clone());
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
            ledger().forget(&device);
            return Ok(Json(ticket));
        }
        Err(error) => error,
    };
    let status = crate::routes::status_for(&error);
    let attempt = if cfg.provider == ProviderKind::Proview
        && router.source() == ProviderSource::Cloud
        && worth_walking(&error)
    {
        Some(diagnose(cfg, &vendor, &device, channel, high).await)
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
        body.stage = Some(attempt.stage);
        body.detail = Some(attempt.detail);
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
        attempts: ledger().attempts(),
    }))
}

#[cfg(test)]
mod tests {
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
    fn the_failing_stage_is_the_first_one_without_a_pass() {
        assert_eq!(first_unproven(&[]), "cloud_login");
        assert_eq!(
            first_unproven(&[report("cloud_login", true)]),
            "device_list"
        );
        assert_eq!(
            first_unproven(&[
                report("cloud_login", true),
                report("device_list", true),
                report("status_cluster", false),
            ]),
            "status_cluster"
        );
        let walked = STAGES
            .iter()
            .take(7)
            .map(|stage| report(stage, true))
            .collect::<Vec<_>>();
        assert_eq!(first_unproven(&walked), "first_frame");
    }

    #[test]
    fn a_failed_attempt_keeps_its_stages_and_counts_its_time() {
        let ledger = Ledger::new();
        assert!(ledger.last("cam").is_none());
        ledger.record(Attempt {
            device: "cam".to_string(),
            channel: 1,
            at: unix_ms(),
            stage: "tunnel",
            detail: "the relay refused the session: ErrorNum 403".to_string(),
            elapsed_ms: 41,
            stages: vec![
                report("cloud_login", true),
                report("device_list", true),
                report("status_cluster", true),
                report("rps_access", true),
                report("tunnel", false),
            ],
        });
        assert_eq!(
            ledger.last("cam").map(|attempt| attempt.stage),
            Some("tunnel")
        );
        assert!(ledger.fresh("cam", Duration::from_secs(30)).is_some());
        assert!(ledger.fresh("cam", Duration::ZERO).is_none());
        assert_eq!(ledger.attempts().len(), 1);
        ledger.forget("cam");
        assert!(ledger.attempts().is_empty());
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
        let attempt = Walk {
            cfg: &cfg,
            session: &session,
            device: "fixture-dev-0001",
            channel: 1,
            high: true,
            status_host: "127.0.0.2".to_string(),
        }
        .run()
        .await;
        assert_eq!(attempt.stage, "device_list");
        assert_eq!(attempt.stages.len(), 2);
        assert!(attempt.stages[0].ok);
        assert_eq!(attempt.stages[0].stage, "cloud_login");
        assert!(!attempt.stages[1].ok);
        assert!(!attempt.stages[1].detail.is_empty());
    }

    #[test]
    fn a_session_without_a_token_stops_the_walk_before_any_call() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
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
        let attempt = runtime.block_on(
            Walk {
                cfg: &cfg,
                session: &session,
                device: "fixture-dev-0001",
                channel: 1,
                high: true,
                status_host: "127.0.0.2".to_string(),
            }
            .run(),
        );
        assert_eq!(attempt.stage, "cloud_login");
        assert_eq!(attempt.stages.len(), 1);
        assert!(!attempt.stages[0].ok);
    }
}
