use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use proview_core::apmode::{self, Credentials};
use proview_core::dvrip::client::{require_ok, Connection};
use proview_core::dvrip::msg;
use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::tokio::time::timeout as deadline;
use rocket::State;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{fail, signed_in, Failure};

pub fn routes() -> Vec<rocket::route::Route> {
    routes![plan, provision, provision_status, provision_stop]
}

#[derive(Deserialize)]
pub struct ProvisionBody {
    ssid: String,
    password: String,
    device_password: Option<String>,
    mode: Option<String>,
    rounds: Option<u32>,
}

#[derive(Serialize)]
pub struct PlanBody {
    pub ok: bool,
    pub record_bytes: usize,
    pub words: usize,
    pub symbols_per_round: usize,
    pub trailers_per_round: usize,
    pub datagram_bytes_per_round: usize,
    pub trailer_bytes: usize,
    pub symbol_gap_ms: u64,
    pub round_gap_ms: u64,
}

#[derive(Serialize)]
pub struct DvripReport {
    pub address: String,
    pub elapsed_ms: u128,
    pub session: Option<u32>,
    pub ret: Option<i64>,
    pub read_back: bool,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct BroadcastStatus {
    pub running: bool,
    pub stopped: bool,
    pub rounds: u32,
    pub datagrams: u64,
    pub bytes: u64,
    pub elapsed_ms: u128,
    pub target: String,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct ProvisionReport {
    pub ok: bool,
    pub mode: String,
    pub record_bytes: usize,
    pub words: usize,
    pub dvrip: Option<DvripReport>,
    pub broadcast: Option<BroadcastStatus>,
    pub skipped: Vec<String>,
}

struct Progress {
    running: bool,
    stopped: bool,
    rounds: u32,
    datagrams: u64,
    bytes: u64,
    started: Instant,
    error: Option<String>,
}

impl Progress {
    fn new() -> Self {
        Progress {
            running: true,
            stopped: false,
            rounds: 0,
            datagrams: 0,
            bytes: 0,
            started: Instant::now(),
            error: None,
        }
    }

    fn status(&self, target: SocketAddr) -> BroadcastStatus {
        BroadcastStatus {
            running: self.running,
            stopped: self.stopped,
            rounds: self.rounds,
            datagrams: self.datagrams,
            bytes: self.bytes,
            elapsed_ms: self.started.elapsed().as_millis(),
            target: target.to_string(),
            error: self.error.clone(),
        }
    }
}

struct Job {
    stop: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
}

static JOB: OnceLock<Mutex<Option<Job>>> = OnceLock::new();

fn jobs() -> &'static Mutex<Option<Job>> {
    JOB.get_or_init(|| Mutex::new(None))
}

fn target() -> SocketAddr {
    SocketAddr::new(apmode::CONFIG_TARGET.into(), apmode::CONFIG_PORT)
}

fn credentials(body: &ProvisionBody) -> Result<Credentials<'_>, Failure> {
    if body.ssid.trim().is_empty() {
        return Err(fail(Status::BadRequest, "ssid is required"));
    }
    if body.ssid.len() > apmode::SSID_MAX {
        return Err(fail(
            Status::BadRequest,
            format!(
                "ssid is {} bytes, limit {}",
                body.ssid.len(),
                apmode::SSID_MAX
            ),
        ));
    }
    if body.password.len() > apmode::PASSWORD_MAX {
        return Err(fail(
            Status::BadRequest,
            format!(
                "password is {} bytes, limit {}",
                body.password.len(),
                apmode::PASSWORD_MAX
            ),
        ));
    }
    Ok(Credentials {
        ssid: &body.ssid,
        password: &body.password,
    })
}

fn requested_mode(body: &ProvisionBody) -> Result<&str, Failure> {
    let mode = body.mode.as_deref().unwrap_or("both");
    match mode {
        "both" | "dvrip" | "broadcast" => Ok(mode),
        other => Err(fail(
            Status::BadRequest,
            format!("mode {other} is not one of both, dvrip, broadcast"),
        )),
    }
}

fn dvrip_address() -> String {
    format!("{}:{}", apmode::AP_ADDRESS, apmode::AP_DVRIP_PORT)
}

#[post("/apmode/plan", data = "<body>")]
fn plan(
    body: Json<ProvisionBody>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<PlanBody>, Failure> {
    signed_in(store, cookies)?;
    let creds = credentials(&body)?;
    let plan = creds
        .plan(apmode::CONFIG_TARGET)
        .map_err(|error| fail(Status::BadRequest, error.to_string()))?;
    let symbols = plan.words.len();
    let symbol_gap_ms = apmode::SYMBOL_GAP.as_millis() as usize;
    let trailer_gap_ms = apmode::TRAILER_GAP_MS as usize;
    let trailers = symbols * symbol_gap_ms / trailer_gap_ms;
    let datagram_bytes: usize = plan
        .words
        .iter()
        .map(|word| usize::from(*word))
        .sum::<usize>()
        + trailers * apmode::TRAILER_BYTES;
    Ok(Json(PlanBody {
        ok: true,
        record_bytes: plan.record.len(),
        words: symbols,
        symbols_per_round: symbols,
        trailers_per_round: trailers,
        datagram_bytes_per_round: datagram_bytes,
        trailer_bytes: apmode::TRAILER_BYTES,
        symbol_gap_ms: apmode::SYMBOL_GAP.as_millis() as u64,
        round_gap_ms: apmode::ROUND_GAP.as_millis() as u64,
    }))
}

#[post("/apmode/provision", data = "<body>")]
async fn provision(
    body: Json<ProvisionBody>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<ProvisionReport>, Failure> {
    signed_in(store, cookies)?;
    let creds = credentials(&body)?;
    let mode = requested_mode(&body)?;
    let plan = creds
        .plan(apmode::CONFIG_TARGET)
        .map_err(|error| fail(Status::BadRequest, error.to_string()))?;
    let record_bytes = plan.record.len();
    let words = plan.words.len();
    let mut skipped = Vec::new();

    let dvrip = if mode == "broadcast" {
        skipped.push("dvrip".to_string());
        None
    } else {
        let password = body.device_password.clone().unwrap_or_default();
        Some(apply_dvrip(&dvrip_address(), apmode::AP_USER, &password, &creds).await)
    };

    let broadcast = if mode == "dvrip" {
        skipped.push("broadcast".to_string());
        None
    } else {
        Some(start_broadcast(plan.payload, plan.words, body.rounds)?)
    };

    Ok(Json(ProvisionReport {
        ok: true,
        mode: mode.to_string(),
        record_bytes,
        words,
        dvrip,
        broadcast,
        skipped,
    }))
}

#[get("/apmode/provision")]
fn provision_status(
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<BroadcastStatus>, Failure> {
    signed_in(store, cookies)?;
    let guard = jobs()
        .lock()
        .map_err(|_| fail(Status::InternalServerError, "job lock"))?;
    match guard.as_ref() {
        Some(job) => {
            let progress = job
                .progress
                .lock()
                .map_err(|_| fail(Status::InternalServerError, "progress lock"))?;
            Ok(Json(progress.status(target())))
        }
        None => Err(fail(
            Status::NotFound,
            "no provisioning run has been started",
        )),
    }
}

#[delete("/apmode/provision")]
fn provision_stop(
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<BroadcastStatus>, Failure> {
    signed_in(store, cookies)?;
    let guard = jobs()
        .lock()
        .map_err(|_| fail(Status::InternalServerError, "job lock"))?;
    let job = guard
        .as_ref()
        .ok_or_else(|| fail(Status::NotFound, "no provisioning run has been started"))?;
    job.stop.store(true, Ordering::Relaxed);
    let progress = job
        .progress
        .lock()
        .map_err(|_| fail(Status::InternalServerError, "progress lock"))?;
    Ok(Json(progress.status(target())))
}

fn start_broadcast(
    payload: Vec<u8>,
    words: Vec<u16>,
    rounds: Option<u32>,
) -> Result<BroadcastStatus, Failure> {
    let mut guard = jobs()
        .lock()
        .map_err(|_| fail(Status::InternalServerError, "job lock"))?;
    if let Some(job) = guard.as_ref() {
        let progress = job
            .progress
            .lock()
            .map_err(|_| fail(Status::InternalServerError, "progress lock"))?;
        if progress.running {
            return Err(fail(
                Status::Conflict,
                "a provisioning run is already sending; stop it first",
            ));
        }
    }
    let stop = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(Mutex::new(Progress::new()));
    let status = progress
        .lock()
        .map_err(|_| fail(Status::InternalServerError, "progress lock"))?
        .status(target());
    let thread_stop = Arc::clone(&stop);
    let thread_progress = Arc::clone(&progress);
    let lengths: Vec<usize> = words.iter().map(|word| usize::from(*word)).collect();
    std::thread::Builder::new()
        .name("apmode-provision".to_string())
        .spawn(move || {
            let timing = Timing::app();
            let outcome = transmit(
                &payload,
                &lengths,
                target(),
                timing,
                rounds,
                &thread_stop,
                &thread_progress,
            );
            if let Ok(mut progress) = thread_progress.lock() {
                progress.running = false;
                progress.stopped = thread_stop.load(Ordering::Relaxed);
                if let Err(error) = outcome {
                    progress.error = Some(error.to_string());
                }
            }
        })
        .map_err(|error| fail(Status::InternalServerError, format!("spawn: {error}")))?;
    *guard = Some(Job { stop, progress });
    Ok(status)
}

#[derive(Clone, Copy)]
struct Timing {
    symbol: Duration,
    round: Duration,
    trailer_gap: Duration,
    ceiling: Duration,
}

impl Timing {
    fn app() -> Self {
        Timing {
            symbol: apmode::SYMBOL_GAP,
            round: apmode::ROUND_GAP,
            trailer_gap: Duration::from_millis(apmode::TRAILER_GAP_MS),
            ceiling: Duration::from_secs(180),
        }
    }

    #[cfg(test)]
    fn instant() -> Self {
        Timing {
            symbol: Duration::ZERO,
            round: Duration::ZERO,
            trailer_gap: Duration::ZERO,
            ceiling: Duration::from_secs(5),
        }
    }

    #[cfg(test)]
    fn patient_trailer() -> Self {
        Timing {
            trailer_gap: Duration::from_secs(3600),
            ..Timing::instant()
        }
    }
}

fn transmit(
    payload: &[u8],
    lengths: &[usize],
    target: SocketAddr,
    timing: Timing,
    rounds: Option<u32>,
    stop: &AtomicBool,
    progress: &Mutex<Progress>,
) -> std::io::Result<()> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_broadcast(true)?;
    let started = Instant::now();
    let mut last_trailer = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) || started.elapsed() > timing.ceiling {
            return Ok(());
        }
        for length in lengths {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let datagram = apmode::datagram(payload, *length).ok_or_else(|| {
                std::io::Error::other(format!("symbol {length} exceeds the payload"))
            })?;
            socket.send_to(datagram, target)?;
            {
                let mut state = progress
                    .lock()
                    .map_err(|_| std::io::Error::other("progress lock"))?;
                state.datagrams += 1;
                state.bytes += datagram.len() as u64;
            }
            std::thread::sleep(timing.symbol);
            if last_trailer.elapsed() >= timing.trailer_gap {
                if let Some(trailer) = apmode::trailer(payload) {
                    socket.send_to(trailer, target)?;
                    let mut state = progress
                        .lock()
                        .map_err(|_| std::io::Error::other("progress lock"))?;
                    state.datagrams += 1;
                    state.bytes += trailer.len() as u64;
                }
                last_trailer = Instant::now();
            }
        }
        {
            let mut state = progress
                .lock()
                .map_err(|_| std::io::Error::other("progress lock"))?;
            state.rounds += 1;
            if let Some(limit) = rounds {
                if state.rounds >= limit {
                    return Ok(());
                }
            }
        }
        std::thread::sleep(timing.round);
    }
}

async fn apply_dvrip(
    address: &str,
    user: &str,
    password: &str,
    creds: &Credentials<'_>,
) -> DvripReport {
    let started = Instant::now();
    let mut report = DvripReport {
        address: address.to_string(),
        elapsed_ms: 0,
        session: None,
        ret: None,
        read_back: false,
        error: None,
    };
    let request_timeout = Duration::from_millis(apmode::CONFIG_TIMEOUT_MS);
    let connect_timeout = Duration::from_millis(apmode::CONNECT_TIMEOUT_MS);
    let mut connection = match deadline(connect_timeout, Connection::connect(address)).await {
        Err(_) => {
            report.error = Some(format!(
                "connect timed out after {} ms",
                apmode::CONNECT_TIMEOUT_MS
            ));
            report.elapsed_ms = started.elapsed().as_millis();
            return report;
        }
        Ok(Err(error)) => {
            report.error = Some(format!("connect: {error}"));
            report.elapsed_ms = started.elapsed().as_millis();
            return report;
        }
        Ok(Ok(connection)) => connection,
    };
    let login = match deadline(request_timeout, connection.login(user, password)).await {
        Err(_) => {
            report.error = Some(format!(
                "login timed out after {} ms",
                apmode::CONFIG_TIMEOUT_MS
            ));
            report.elapsed_ms = started.elapsed().as_millis();
            return report;
        }
        Ok(Err(error)) => {
            report.error = Some(format!("login: {error}"));
            report.elapsed_ms = started.elapsed().as_millis();
            return report;
        }
        Ok(Ok(login)) => login,
    };
    report.session = Some(login.session);
    let existing = match deadline(request_timeout, connection.config_get(apmode::CONFIG_NAME)).await
    {
        Ok(Ok(reply)) => apmode::wifi_block(&reply),
        _ => None,
    };
    let wifi = apmode::wifi_config(existing.as_ref(), creds);
    let mut body = serde_json::Map::new();
    body.insert("Name".to_string(), json!(apmode::CONFIG_NAME));
    body.insert(
        "SessionID".to_string(),
        json!(format!("0x{:08X}", connection.session())),
    );
    body.insert(apmode::CONFIG_NAME.to_string(), wifi);
    let body = Value::Object(body);
    match deadline(request_timeout, connection.request(msg::CONFIG_SET, body)).await {
        Err(_) => {
            report.error = Some(format!(
                "set timed out after {} ms",
                apmode::CONFIG_TIMEOUT_MS
            ));
            report.elapsed_ms = started.elapsed().as_millis();
            return report;
        }
        Ok(Err(error)) => {
            report.error = Some(format!("set: {error}"));
            report.elapsed_ms = started.elapsed().as_millis();
            return report;
        }
        Ok(Ok(reply)) => match require_ok(&reply) {
            Ok(ret) => report.ret = Some(ret),
            Err(error) => {
                report.error = Some(format!("set: {error}"));
                report.elapsed_ms = started.elapsed().as_millis();
                return report;
            }
        },
    }
    report.read_back =
        match deadline(request_timeout, connection.config_get(apmode::CONFIG_NAME)).await {
            Ok(Ok(reply)) => apmode::wifi_block(&reply)
                .and_then(|wifi| {
                    wifi.get("SSID")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                })
                .map(|ssid| ssid == creds.ssid)
                .unwrap_or(false),
            _ => false,
        };
    let _ = deadline(request_timeout, connection.logout()).await;
    report.elapsed_ms = started.elapsed().as_millis();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn progress() -> Mutex<Progress> {
        Mutex::new(Progress::new())
    }

    #[test]
    fn one_round_puts_every_symbol_and_the_trailer_on_the_wire() {
        let receiver = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let plan = Credentials {
            ssid: "CAM-LAB",
            password: "rahasia123",
        }
        .plan(Ipv4Addr::BROADCAST)
        .unwrap();
        let lengths: Vec<usize> = plan.words.iter().map(|word| usize::from(*word)).collect();
        let stop = AtomicBool::new(false);
        let state = progress();
        transmit(
            &plan.payload,
            &lengths,
            receiver.local_addr().unwrap(),
            Timing::instant(),
            Some(1),
            &stop,
            &state,
        )
        .unwrap();
        let mut buffer = [0u8; 2048];
        let mut seen = Vec::new();
        while let Ok((size, _)) = receiver.recv_from(&mut buffer) {
            seen.push((size, buffer[..size.min(16)].to_vec()));
        }
        assert_eq!(seen.len(), lengths.len() * 2);
        for (index, (size, head)) in seen.iter().enumerate() {
            if index % 2 == 0 {
                assert_eq!(*size, lengths[index / 2]);
                assert_eq!(&head[..10], &[apmode::PAYLOAD_FILL; 10]);
            } else {
                assert_eq!(*size, apmode::TRAILER_BYTES);
            }
        }
        let progress = state.lock().unwrap();
        assert_eq!(progress.rounds, 1);
        assert_eq!(progress.datagrams as usize, lengths.len() * 2);
    }

    #[test]
    fn a_long_trailer_gap_leaves_the_symbols_alone() {
        let receiver = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let plan = Credentials {
            ssid: "CAM",
            password: "",
        }
        .plan(Ipv4Addr::BROADCAST)
        .unwrap();
        let lengths: Vec<usize> = plan.words.iter().map(|word| usize::from(*word)).collect();
        let stop = AtomicBool::new(false);
        let state = progress();
        transmit(
            &plan.payload,
            &lengths,
            receiver.local_addr().unwrap(),
            Timing::patient_trailer(),
            Some(1),
            &stop,
            &state,
        )
        .unwrap();
        let mut buffer = [0u8; 2048];
        let mut seen = Vec::new();
        while let Ok((size, _)) = receiver.recv_from(&mut buffer) {
            seen.push(size);
        }
        assert_eq!(seen, lengths);
    }

    #[test]
    fn the_stop_flag_ends_the_run_between_rounds() {
        let receiver = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let plan = Credentials {
            ssid: "CAM",
            password: "",
        }
        .plan(Ipv4Addr::BROADCAST)
        .unwrap();
        let lengths: Vec<usize> = plan.words.iter().map(|word| usize::from(*word)).collect();
        let stop = AtomicBool::new(true);
        let state = progress();
        transmit(
            &plan.payload,
            &lengths,
            receiver.local_addr().unwrap(),
            Timing::instant(),
            None,
            &stop,
            &state,
        )
        .unwrap();
        let progress = state.lock().unwrap();
        assert_eq!(progress.rounds, 0);
        assert_eq!(progress.datagrams, 0);
    }

    #[tokio::test]
    async fn a_device_that_never_answers_is_given_up_on_after_the_request_timeout() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let _held = stream;
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        });
        let creds = Credentials {
            ssid: "CAM",
            password: "x",
        };
        let started = Instant::now();
        let report = apply_dvrip(&address, apmode::AP_USER, "", &creds).await;
        let elapsed = started.elapsed();
        assert!(report.error.is_some());
        assert!(report.session.is_none());
        assert!(
            elapsed >= Duration::from_millis(apmode::CONFIG_TIMEOUT_MS),
            "gave up in {elapsed:?}, before the timeout"
        );
        assert!(
            elapsed < Duration::from_millis(apmode::CONFIG_TIMEOUT_MS + 1500),
            "took {elapsed:?}, past the timeout"
        );
    }
}
