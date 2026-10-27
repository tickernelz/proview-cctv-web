use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};

use proview_core::apmode::{self, Credentials, Plan};
use proview_core::config::Config;
use proview_core::dvrip::client::{require_ok, Connection};
use proview_core::dvrip::msg;
use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{fail, session_id, Failure};

pub fn routes() -> Vec<rocket::route::Route> {
    routes![plan, provision]
}

#[derive(Deserialize)]
pub struct ProvisionBody {
    ssid: String,
    password: String,
    host: Option<String>,
    target: Option<String>,
    port: Option<u16>,
    mode: Option<String>,
    rounds: Option<u32>,
}

#[derive(Serialize)]
pub struct PlanBody {
    pub ok: bool,
    pub info: String,
    pub record_hex: String,
    pub record_bytes: usize,
    pub words: usize,
    pub packet_bytes: usize,
    pub datagrams_per_round: usize,
    pub datagram_bytes_per_round: usize,
    pub trailer_bytes: usize,
    pub rounds: u32,
}

#[derive(Serialize)]
pub struct DvripReport {
    pub address: String,
    pub session: u32,
    pub ret: i64,
    pub read_back: bool,
}

#[derive(Serialize)]
pub struct SendReport {
    pub target: String,
    pub port: u16,
    pub rounds: u32,
    pub datagrams: usize,
    pub bytes: usize,
}

#[derive(Serialize)]
pub struct ProvisionReport {
    pub ok: bool,
    pub ssid: String,
    pub info: String,
    pub record_hex: String,
    pub dvrip: Option<DvripReport>,
    pub broadcast: Option<SendReport>,
    pub skipped: Vec<String>,
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

fn destination(body: &ProvisionBody) -> Result<Ipv4Addr, Failure> {
    match body.target.as_deref() {
        Some(raw) => raw.trim().parse().map_err(|_| {
            fail(
                Status::BadRequest,
                format!("target {raw} is not an IPv4 address"),
            )
        }),
        None => Ok(Ipv4Addr::BROADCAST),
    }
}

fn rounds(body: &ProvisionBody) -> u32 {
    body.rounds.unwrap_or(apmode::ROUNDS).clamp(1, 200)
}

fn dvrip_address(body: &ProvisionBody) -> String {
    body.host
        .clone()
        .unwrap_or_else(|| format!("{}:{}", apmode::AP_ADDRESS, apmode::AP_DVRIP_PORT))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn plan_body(plan: &Plan, rounds: u32) -> PlanBody {
    PlanBody {
        ok: true,
        info: plan.info.clone(),
        record_hex: hex(&plan.record),
        record_bytes: plan.record.len(),
        words: plan.words.len(),
        packet_bytes: plan.packet.len(),
        datagrams_per_round: plan.datagram_bytes.len() + 1,
        datagram_bytes_per_round: plan.datagram_bytes.iter().sum::<usize>() + plan.trailer_bytes,
        trailer_bytes: plan.trailer_bytes,
        rounds,
    }
}

#[post("/apmode/plan", data = "<body>")]
fn plan(
    body: Json<ProvisionBody>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<PlanBody>, Failure> {
    session_id_check(store, cookies)?;
    let creds = credentials(&body)?;
    let plan = creds
        .plan(destination(&body)?)
        .map_err(|e| fail(Status::BadRequest, e.to_string()))?;
    Ok(Json(plan_body(&plan, rounds(&body))))
}

#[post("/apmode/provision", data = "<body>")]
async fn provision(
    body: Json<ProvisionBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<ProvisionReport>, Failure> {
    session_id_check(store, cookies)?;
    let creds = credentials(&body)?;
    let destination = destination(&body)?;
    let plan = creds
        .plan(destination)
        .map_err(|e| fail(Status::BadRequest, e.to_string()))?;
    let mode = body.mode.as_deref().unwrap_or("both");
    if !matches!(mode, "both" | "dvrip" | "broadcast") {
        return Err(fail(
            Status::BadRequest,
            format!("mode {mode} is not one of both, dvrip, broadcast"),
        ));
    }

    let mut skipped = Vec::new();
    let dvrip = if mode == "broadcast" {
        skipped.push("dvrip".to_string());
        None
    } else {
        Some(apply_dvrip(&body, &cfg.device_user, &cfg.device_pass, &creds).await?)
    };

    let broadcast = if mode == "dvrip" {
        skipped.push("broadcast".to_string());
        None
    } else {
        let rounds = rounds(&body);
        let window = plan.window.clone();
        let lengths = plan.datagram_bytes.clone();
        let port = body.port.unwrap_or(apmode::CONFIG_PORT);
        let target = SocketAddr::new(destination.into(), port);
        let report = rocket::tokio::task::spawn_blocking(move || {
            transmit(&window, &lengths, target, rounds)
        })
        .await
        .map_err(|e| fail(Status::InternalServerError, format!("transmit task: {e}")))?
        .map_err(|e| fail(Status::BadGateway, format!("udp {target}: {e}")))?;
        Some(report)
    };

    Ok(Json(ProvisionReport {
        ok: true,
        ssid: body.ssid.clone(),
        info: plan.info,
        record_hex: hex(&plan.record),
        dvrip,
        broadcast,
        skipped,
    }))
}

fn session_id_check(store: &State<Store>, cookies: &CookieJar<'_>) -> Result<(), Failure> {
    let id = session_id(cookies)?;
    store
        .get(&id)
        .map(|_| ())
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))
}

async fn apply_dvrip(
    body: &ProvisionBody,
    user: &str,
    pass: &str,
    creds: &Credentials<'_>,
) -> Result<DvripReport, Failure> {
    let address = dvrip_address(body);
    let mut connection = Connection::connect(&address)
        .await
        .map_err(|e| fail(Status::BadGateway, format!("connect {address}: {e}")))?;
    let login = connection
        .login(user, pass)
        .await
        .map_err(|e| fail(Status::BadGateway, format!("login {address}: {e}")))?;
    let existing = connection
        .config_get(apmode::CONFIG_NAME)
        .await
        .ok()
        .and_then(|reply| apmode::wifi_block(&reply));
    let wifi = apmode::wifi_config(existing.as_ref(), creds);
    let reply = connection
        .request(
            msg::CONFIG_SET,
            json!({
                "Name": apmode::CONFIG_NAME,
                "SessionID": format!("0x{:08X}", connection.session()),
                "NetWork": { "Wifi": wifi },
            }),
        )
        .await
        .map_err(|e| fail(Status::BadGateway, format!("set {address}: {e}")))?;
    let ret =
        require_ok(&reply).map_err(|e| fail(Status::BadGateway, format!("set {address}: {e}")))?;
    let read_back = connection
        .config_get(apmode::CONFIG_NAME)
        .await
        .ok()
        .and_then(|reply| apmode::wifi_block(&reply))
        .and_then(|wifi| {
            wifi.get("SSID")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .map(|ssid| ssid == body.ssid)
        .unwrap_or(false);
    let session = login.session;
    let _ = connection.logout().await;
    Ok(DvripReport {
        address,
        session,
        ret,
        read_back,
    })
}

fn transmit(
    window: &[u8],
    lengths: &[usize],
    target: SocketAddr,
    rounds: u32,
) -> std::io::Result<SendReport> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_broadcast(true)?;
    let sent = AtomicUsize::new(0);
    let bytes = AtomicUsize::new(0);
    for _ in 0..rounds {
        for length in lengths {
            let payload = apmode::datagram(window, *length)
                .ok_or_else(|| std::io::Error::other("datagram slice out of range"))?;
            socket.send_to(payload, target)?;
            sent.fetch_add(1, Ordering::Relaxed);
            bytes.fetch_add(payload.len(), Ordering::Relaxed);
            std::thread::sleep(apmode::SYMBOL_GAP);
        }
        let trailer = apmode::trailer(window)
            .ok_or_else(|| std::io::Error::other("trailer slice out of range"))?;
        socket.send_to(trailer, target)?;
        sent.fetch_add(1, Ordering::Relaxed);
        bytes.fetch_add(trailer.len(), Ordering::Relaxed);
        std::thread::sleep(apmode::ROUND_GAP);
    }
    Ok(SendReport {
        target: target.ip().to_string(),
        port: target.port(),
        rounds,
        datagrams: sent.into_inner(),
        bytes: bytes.into_inner(),
    })
}
