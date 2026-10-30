use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use proview_core::config::Config;
use proview_core::device::{op_window, ControlError, DvripControl, SESSION_WINDOW};
use proview_core::dvrip::client::{Connection, DvrError};
use proview_core::dvrip::control::{PresetEntry, TourLine};
use proview_core::dvrip::ptz::{
    ability_refusal, locate_offsets, next_preset_id, preset_name, PanoramaRange, PtzAction,
    PtzOrientation, PtzSpeed, CALIBRATE_ABILITY, LOCATE_ABILITY, PRESET_ABILITY, PRESET_LIMIT,
    ZOOM_ABILITY,
};
use proview_core::provider::cloud::CloudProvider;
use proview_core::session::Store;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;
use tokio::sync::oneshot;

use super::control::{control_failure, controls_for};
use super::{fail, session_id, Failure};
use crate::config::ModeStore;

pub const HOLD_LIMIT: Duration = Duration::from_secs(30);
const HOLD_ENV: &str = "PROVIEW_PTZ_HOLD_MS";
pub const CALIBRATE_WINDOW: Duration = Duration::from_secs(60);
const RELEASE_MARGIN: Duration = Duration::from_secs(2);
pub const PARK_WINDOW: Duration = Duration::from_secs(20);
pub const REUSE_WINDOW: Duration = Duration::from_millis(1500);

pub fn routes() -> Vec<rocket::route::Route> {
    routes![
        ptz_press,
        ptz_release,
        ptz_locate,
        preset_add,
        preset_rename,
        preset_delete,
        tour_remove,
        ptz_calibrate,
        panorama_read,
        panorama_goto,
    ]
}

fn ms(duration: Duration) -> u64 {
    duration.as_millis() as u64
}

fn timed_out(operation: &'static str, control: &DvripControl, window: Duration) -> ControlError {
    ControlError::Timeout {
        operation,
        address: control.address(),
        window,
    }
}

async fn within<T, F>(
    operation: &'static str,
    control: &DvripControl,
    window: Duration,
    call: F,
) -> Result<T, ControlError>
where
    F: Future<Output = Result<T, DvrError>>,
{
    match tokio::time::timeout(window, call).await {
        Ok(result) => result.map_err(|error| match error {
            DvrError::Rejected { ret, reason } => ControlError::Device(format!(
                "{operation}: the camera refused with Ret {ret} ({reason})"
            )),
            other => ControlError::Device(format!("{operation}: {other}")),
        }),
        Err(_) => Err(timed_out(operation, control, window)),
    }
}

struct Opened {
    connection: Connection,
    session_ms: u64,
}

async fn open(control: &DvripControl) -> Result<Opened, ControlError> {
    let started = Instant::now();
    let connection = control.session_within(SESSION_WINDOW).await?;
    Ok(Opened {
        connection,
        session_ms: ms(started.elapsed()),
    })
}

async fn finish<T>(
    mut connection: Connection,
    result: Result<T, ControlError>,
) -> Result<T, ControlError> {
    if !matches!(result, Err(ControlError::Timeout { .. })) {
        let _ = connection.logout_within(op_window()).await;
    }
    result
}

pub fn hold_limit() -> Duration {
    match std::env::var(HOLD_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
    {
        Some(ms) if ms > 0 => Duration::from_millis(ms),
        _ => HOLD_LIMIT,
    }
}

async fn refusal(
    control: &DvripControl,
    connection: &mut Connection,
    path: &[&str],
) -> Result<Option<String>, ControlError> {
    let abilities = within("ability read", control, op_window(), connection.abilities()).await?;
    Ok(ability_refusal(&abilities, path))
}

fn unsupported(operation: &str, reason: String) -> Failure {
    fail(
        Status::NotImplemented,
        format!("{operation}: {reason}, so the app hides this control and nothing was sent to the camera"),
    )
}

async fn gate(
    control: &DvripControl,
    mut connection: Connection,
    path: &[&str],
    operation: &str,
) -> Result<Connection, Failure> {
    match refusal(control, &mut connection, path).await {
        Ok(None) => Ok(connection),
        Ok(Some(reason)) => {
            let _ = connection.logout_within(op_window()).await;
            Err(unsupported(operation, reason))
        }
        Err(error) => Err(control_failure(
            finish::<()>(connection, Err(error)).await.unwrap_err(),
        )),
    }
}

pub(crate) async fn tour_gate(control: &DvripControl, operation: &str) -> Result<(), Failure> {
    let Opened { connection, .. } = open(control).await.map_err(control_failure)?;
    let mut connection = gate(control, connection, &PRESET_ABILITY, operation).await?;
    let _ = connection.logout_within(op_window()).await;
    Ok(())
}

fn dvrip_of(
    device: Option<&str>,
    cfg: &Config,
    cloud: &Arc<CloudProvider>,
    modes: &ModeStore,
    store: &Store,
    cookies: &CookieJar<'_>,
) -> Result<DvripControl, Failure> {
    let controls = controls_for(device, cfg, cloud, modes, store, cookies)?;
    controls.dvrip().cloned().map_err(control_failure)
}

type ReleaseReply = oneshot::Sender<Result<Option<ReleaseTimes>, ControlError>>;

struct Held {
    generation: u64,
    action: PtzAction,
    release: oneshot::Sender<Release>,
}

struct Release {
    asked: Instant,
    pressed_for: Option<Duration>,
    reply: ReleaseReply,
}

fn held() -> &'static Mutex<HashMap<String, Held>> {
    static HELD: OnceLock<Mutex<HashMap<String, Held>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_generation() -> u64 {
    static GENERATION: AtomicU64 = AtomicU64::new(1);
    GENERATION.fetch_add(1, Ordering::Relaxed)
}

fn hold_key(cookies: &CookieJar<'_>, device: Option<&str>) -> Result<String, Failure> {
    let session = session_id(cookies)?;
    Ok(format!(
        "{session}|{}",
        device.map(str::trim).unwrap_or_default()
    ))
}

fn take_held(key: &str) -> Option<Held> {
    held().lock().ok()?.remove(key)
}

fn forget_held(key: &str, generation: u64) {
    if let Ok(mut map) = held().lock() {
        if map.get(key).map(|entry| entry.generation) == Some(generation) {
            map.remove(key);
        }
    }
}

#[derive(Debug, Clone)]
struct PressTimes {
    session_ms: u64,
    press_ms: u64,
    reused: bool,
    sent: PtzAction,
    orientation: Option<PtzOrientation>,
}

struct Parked {
    generation: u64,
    connection: Connection,
}

fn parked() -> &'static Mutex<HashMap<String, Parked>> {
    static PARKED: OnceLock<Mutex<HashMap<String, Parked>>> = OnceLock::new();
    PARKED.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn parked_key(account: &str, address: &str) -> String {
    format!("{account}|{address}")
}

fn take_parked(key: &str) -> Option<Connection> {
    parked()
        .lock()
        .ok()?
        .remove(key)
        .map(|entry| entry.connection)
}

fn park(address: String, connection: Connection) {
    let generation = next_generation();
    let previous = match parked().lock() {
        Ok(mut map) => map.insert(
            address.clone(),
            Parked {
                generation,
                connection,
            },
        ),
        Err(_) => return,
    };
    tokio::spawn(async move {
        if let Some(mut previous) = previous {
            let _ = previous.connection.logout_within(op_window()).await;
        }
        tokio::time::sleep(PARK_WINDOW).await;
        let expired = parked().lock().ok().and_then(|mut map| {
            if map.get(&address).map(|entry| entry.generation) == Some(generation) {
                map.remove(&address)
            } else {
                None
            }
        });
        if let Some(mut expired) = expired {
            let _ = expired.connection.logout_within(op_window()).await;
        }
    });
}

enum Started {
    Pressed {
        connection: Connection,
        sent: PtzAction,
        orientation: Option<PtzOrientation>,
        press_ms: u64,
    },
    Refused(Failure),
    Failed {
        connection: Connection,
        error: ControlError,
        sent: PtzAction,
        press_timed_out: bool,
    },
}

async fn start(
    control: &DvripControl,
    mut connection: Connection,
    gated: bool,
    action: PtzAction,
    step: u8,
    window: Duration,
) -> Started {
    let failed = |connection, error, press_timed_out| Started::Failed {
        connection,
        error,
        sent: action,
        press_timed_out,
    };
    let orientation = if gated {
        let abilities = within("ability read", control, window, connection.abilities()).await;
        match abilities {
            Ok(abilities) => {
                if let Some(reason) = ability_refusal(&abilities, &ZOOM_ABILITY) {
                    let _ = connection.logout_within(op_window()).await;
                    return Started::Refused(unsupported("ptz press", reason));
                }
            }
            Err(error) => return failed(connection, error, false),
        }
        None
    } else {
        let address = control.address();
        match within(
            "ptz orientation read",
            control,
            window,
            connection.ptz_orientation(&address),
        )
        .await
        {
            Ok(orientation) => Some(orientation),
            Err(error) => return failed(connection, error, false),
        }
    };
    let sent = match &orientation {
        Some(orientation) => action.oriented(orientation),
        None => action,
    };
    let press_started = Instant::now();
    match within(
        "ptz press",
        control,
        window,
        connection.ptz_press(sent, step),
    )
    .await
    {
        Ok(()) => Started::Pressed {
            connection,
            sent,
            orientation,
            press_ms: ms(press_started.elapsed()),
        },
        Err(error) => {
            let timed = matches!(error, ControlError::Timeout { .. });
            Started::Failed {
                connection,
                error,
                sent,
                press_timed_out: timed,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ReleaseTimes {
    pub held_ms: u64,
    pub stop_ms: u64,
    pub fresh_session: bool,
}

async fn send_stop(
    control: &DvripControl,
    connection: Option<Connection>,
    action: PtzAction,
    step: u8,
) -> Result<(u64, bool, Option<Connection>), ControlError> {
    let window = op_window();
    if let Some(mut connection) = connection {
        let started = Instant::now();
        let sent = within(
            "ptz release",
            control,
            window,
            connection.ptz_release(action, step),
        )
        .await;
        if sent.is_ok() {
            return Ok((ms(started.elapsed()), false, Some(connection)));
        }
    }
    let started = Instant::now();
    let Opened { connection, .. } = open(control).await?;
    let mut connection = connection;
    let sent = within(
        "ptz release",
        control,
        window,
        connection.ptz_release(action, step),
    )
    .await;
    finish(connection, sent).await?;
    Ok((ms(started.elapsed()), true, None))
}

fn answer_unpressed(release: &mut oneshot::Receiver<Release>) {
    if let Ok(request) = release.try_recv() {
        let _ = request.reply.send(Ok(None));
    }
}

async fn stop_orphan(control: &DvripControl, pending: Option<PtzAction>, step: u8) {
    let Some(sent) = pending else {
        return;
    };
    match send_stop(control, None, sent, step).await {
        Ok(_) => {
            tracing::warn!(device = %control.address(), "a parked ptz press timed out and the fresh press did not go out, so a stop went out on its own session")
        }
        Err(error) => {
            tracing::error!(device = %control.address(), %error, "a parked ptz press timed out and its stop did not go out")
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn hold(
    control: DvripControl,
    account: String,
    begun: Instant,
    gated: bool,
    action: PtzAction,
    step: u8,
    started: oneshot::Sender<Result<PressTimes, Failure>>,
    mut release: oneshot::Receiver<Release>,
    finished: impl FnOnce(),
) {
    let address = control.address();
    let parking = parked_key(&account, &address);
    let mut warm = None;
    let mut pending = None;
    if let Some(connection) = take_parked(&parking) {
        match start(&control, connection, gated, action, step, REUSE_WINDOW).await {
            Started::Failed {
                connection,
                error,
                sent,
                press_timed_out,
            } => {
                tracing::info!(device = %address, %error, "a parked ptz session failed, so the press opens a fresh one");
                drop(connection);
                if press_timed_out {
                    pending = Some(sent);
                }
            }
            other => warm = Some(other),
        }
    }
    let reused = warm.is_some();
    let (outcome, session_ms) = match warm {
        Some(outcome) => (outcome, 0),
        None => {
            let Opened {
                connection,
                session_ms,
            } = match open(&control).await {
                Ok(opened) => opened,
                Err(error) => {
                    finished();
                    let _ = started.send(Err(control_failure(error)));
                    answer_unpressed(&mut release);
                    stop_orphan(&control, pending, step).await;
                    return;
                }
            };
            (
                start(&control, connection, gated, action, step, op_window()).await,
                session_ms,
            )
        }
    };
    let (connection, action) = match outcome {
        Started::Pressed {
            connection,
            sent,
            orientation,
            press_ms,
        } => {
            let _ = started.send(Ok(PressTimes {
                session_ms,
                press_ms,
                reused,
                sent,
                orientation,
            }));
            (connection, sent)
        }
        Started::Refused(failure) => {
            finished();
            let _ = started.send(Err(failure));
            answer_unpressed(&mut release);
            stop_orphan(&control, pending, step).await;
            return;
        }
        Started::Failed {
            mut connection,
            error,
            sent,
            press_timed_out,
        } => {
            let timed = matches!(error, ControlError::Timeout { .. });
            finished();
            let _ = started.send(Err(control_failure(error)));
            answer_unpressed(&mut release);
            if press_timed_out {
                drop(connection);
                if let Err(error) = send_stop(&control, None, sent, step).await {
                    tracing::error!(device = %address, %error, "a ptz press that timed out did not get its stop out");
                }
            } else {
                if !timed {
                    let _ = connection.logout_within(op_window()).await;
                }
                stop_orphan(&control, pending, step).await;
            }
            return;
        }
    };
    let request = tokio::select! {
        request = &mut release => request.ok(),
        _ = tokio::time::sleep(hold_limit()) => None,
    };
    finished();
    let (held_ms, reply) = match request {
        Some(Release {
            asked,
            pressed_for,
            reply,
        }) => {
            let pressed_for = pressed_for.unwrap_or_else(|| asked.saturating_duration_since(begun));
            (ms(pressed_for), Some(reply))
        }
        None => (ms(hold_limit()), None),
    };
    let outcome = send_stop(&control, Some(connection), action, step)
        .await
        .map(|(stop_ms, fresh_session, connection)| {
            if let Some(connection) = connection {
                park(parking.clone(), connection);
            }
            ReleaseTimes {
                held_ms,
                stop_ms,
                fresh_session,
            }
        });
    match reply {
        Some(reply) => {
            let _ = reply.send(outcome.map(Some));
        }
        None => tracing::warn!(
            device = %control.address(),
            action = action.name(),
            stopped = outcome.is_ok(),
            "a ptz hold reached its limit without a release and was stopped"
        ),
    }
}

async fn release_entry(
    entry: Held,
    pressed_for: Option<Duration>,
) -> Result<Option<ReleaseTimes>, ControlError> {
    let (reply, answer) = oneshot::channel();
    let request = Release {
        asked: Instant::now(),
        pressed_for,
        reply,
    };
    if entry.release.send(request).is_err() {
        return Ok(None);
    }
    let bound = SESSION_WINDOW + op_window() * 2 + RELEASE_MARGIN + hold_limit();
    match tokio::time::timeout(bound, answer).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(_)) => Ok(None),
        Err(_) => Err(ControlError::Device(
            "the held move ended without reporting its stop".to_string(),
        )),
    }
}

#[derive(Serialize)]
pub struct PressBody {
    pub ok: bool,
    pub backend: String,
    pub action: String,
    pub command: String,
    pub speed: PtzSpeed,
    pub step: u8,
    pub orientation: Option<PtzOrientation>,
    pub reused_session: bool,
    pub session_ms: u64,
    pub press_ms: u64,
    pub limit_ms: u64,
}

#[allow(clippy::too_many_arguments)]
#[post("/device/controls/ptz/press?<action>&<speed>&<device>")]
async fn ptz_press(
    action: String,
    speed: Option<String>,
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<PressBody>, Failure> {
    let parsed = PtzAction::parse(&action).ok_or_else(|| {
        fail(
            Status::BadRequest,
            "action must be up, down, left, right, up-left, up-right, down-left, down-right, zoom-in, zoom-out, focus-near or focus-far",
        )
    })?;
    let speed = match speed.as_deref() {
        None => PtzSpeed::default(),
        Some(raw) => PtzSpeed::parse(raw)
            .ok_or_else(|| fail(Status::BadRequest, "speed must be slow, normal or fast"))?,
    };
    let step = parsed.step(speed);
    let key = hold_key(cookies, device.as_deref())?;
    let account = store
        .get(&session_id(cookies)?)
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))?
        .account;
    let control = dvrip_of(device.as_deref(), cfg, cloud, modes, store, cookies)?;
    if let Some(previous) = take_held(&key) {
        let _ = release_entry(previous, None).await;
    }
    let begun = Instant::now();
    let generation = next_generation();
    let (release_tx, release_rx) = oneshot::channel();
    let (started_tx, started_rx) = oneshot::channel();
    if let Ok(mut map) = held().lock() {
        map.insert(
            key.clone(),
            Held {
                generation,
                action: parsed,
                release: release_tx,
            },
        );
    }
    let owned_key = key.clone();
    tokio::spawn(hold(
        control,
        account,
        begun,
        parsed.moves_the_lens(),
        parsed,
        step,
        started_tx,
        release_rx,
        move || forget_held(&owned_key, generation),
    ));
    let times = match started_rx.await {
        Ok(Ok(times)) => times,
        Ok(Err(failure)) => return Err(failure),
        Err(_) => {
            return Err(fail(
                Status::BadGateway,
                "ptz press: the hold ended before it reported the press",
            ))
        }
    };
    Ok(Json(PressBody {
        ok: true,
        backend: "dvrip".to_string(),
        action: parsed.name().to_string(),
        command: times.sent.wire().to_string(),
        speed,
        step,
        orientation: times.orientation,
        reused_session: times.reused,
        session_ms: times.session_ms,
        press_ms: times.press_ms,
        limit_ms: ms(hold_limit()),
    }))
}

#[derive(Serialize)]
pub struct ReleaseBody {
    pub ok: bool,
    pub held: bool,
    pub action: Option<String>,
    pub times: Option<ReleaseTimes>,
    pub detail: String,
}

#[post("/device/controls/ptz/release?<pressed_ms>&<device>")]
async fn ptz_release(
    pressed_ms: Option<u64>,
    device: Option<String>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<ReleaseBody>, Failure> {
    let id = session_id(cookies)?;
    store
        .get(&id)
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))?;
    let key = hold_key(cookies, device.as_deref())?;
    let Some(entry) = take_held(&key) else {
        return Ok(Json(ReleaseBody {
            ok: true,
            held: false,
            action: None,
            times: None,
            detail: "nothing was held for this camera, so no stop went out".to_string(),
        }));
    };
    let action = entry.action;
    let Some(times) = release_entry(entry, pressed_ms.map(Duration::from_millis))
        .await
        .map_err(control_failure)?
    else {
        return Ok(Json(ReleaseBody {
            ok: true,
            held: false,
            action: Some(action.name().to_string()),
            times: None,
            detail: "the press never reached the camera, so no stop was needed".to_string(),
        }));
    };
    Ok(Json(ReleaseBody {
        ok: true,
        held: true,
        action: Some(action.name().to_string()),
        detail: format!(
            "{} stopped after {} ms held (stop ack {} ms)",
            action.name(),
            times.held_ms,
            times.stop_ms
        ),
        times: Some(times),
    }))
}

#[derive(Serialize)]
pub struct LocateBody {
    pub ok: bool,
    pub xoffset: i32,
    pub yoffset: i32,
    pub session_ms: u64,
    pub locate_ms: u64,
}

#[allow(clippy::too_many_arguments)]
#[post("/device/controls/ptz/locate?<x>&<y>&<device>")]
async fn ptz_locate(
    x: f64,
    y: f64,
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<LocateBody>, Failure> {
    let (xoffset, yoffset) = locate_offsets(x, y).ok_or_else(|| {
        fail(
            Status::BadRequest,
            "x and y are the click position inside the wide lens picture, each from 0 to 1",
        )
    })?;
    let control = dvrip_of(device.as_deref(), cfg, cloud, modes, store, cookies)?;
    let Opened {
        connection,
        session_ms,
    } = open(&control).await.map_err(control_failure)?;
    let mut connection = gate(&control, connection, &LOCATE_ABILITY, "ptz locate").await?;
    let started = Instant::now();
    let sent = within(
        "ptz locate",
        &control,
        op_window(),
        connection.ptz_locate_to(xoffset, yoffset),
    )
    .await;
    let locate_ms = ms(started.elapsed());
    finish(connection, sent).await.map_err(control_failure)?;
    Ok(Json(LocateBody {
        ok: true,
        xoffset,
        yoffset,
        session_ms,
        locate_ms,
    }))
}

#[derive(Serialize)]
pub struct PresetEditBody {
    pub ok: bool,
    pub action: String,
    pub id: i64,
    pub name: String,
    pub presets: Vec<PresetEntry>,
    pub lines: Vec<TourLine>,
    pub session_ms: u64,
    pub command_ms: u64,
}

fn listing_has(presets: &[PresetEntry], id: i64) -> bool {
    presets.iter().any(|preset| preset.id == id)
}

fn preset_id(token: &str) -> Result<i64, Failure> {
    token
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| {
            fail(
                Status::BadRequest,
                "token must be a preset id listed under Uart.PTZPreset",
            )
        })
}

async fn listing(
    control: &DvripControl,
    connection: &mut Connection,
    operation: &'static str,
) -> Result<(Vec<TourLine>, Vec<PresetEntry>), ControlError> {
    within(operation, control, op_window(), connection.tour_listing()).await
}

#[allow(clippy::too_many_arguments)]
#[post("/device/controls/preset/add?<name>&<device>")]
async fn preset_add(
    name: Option<String>,
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<PresetEditBody>, Failure> {
    let control = dvrip_of(device.as_deref(), cfg, cloud, modes, store, cookies)?;
    let Opened {
        connection,
        session_ms,
    } = open(&control).await.map_err(control_failure)?;
    let mut connection = gate(&control, connection, &PRESET_ABILITY, "preset add").await?;
    let outcome = async {
        let (_, before) = listing(&control, &mut connection, "preset listing").await?;
        if before.len() >= PRESET_LIMIT {
            return Ok(Err(fail(
                Status::Conflict,
                format!("Limit reached: the camera already holds {} presets, the app allows {PRESET_LIMIT}", before.len()),
            )));
        }
        let id = next_preset_id(&before);
        let label = preset_name(name.as_deref(), id);
        let started = Instant::now();
        within(
            "preset set",
            &control,
            op_window(),
            connection.set_preset(id, &label),
        )
        .await?;
        let command_ms = ms(started.elapsed());
        let (lines, presets) = listing(&control, &mut connection, "preset read back").await?;
        Ok(Ok((id, label, lines, presets, command_ms)))
    }
    .await;
    let (id, label, lines, presets, command_ms) = finish(connection, outcome)
        .await
        .map_err(control_failure)??;
    if !listing_has(&presets, id) {
        return Err(fail(
            Status::BadGateway,
            format!(
                "preset add: the camera acked SetPreset {id} but Uart.PTZPreset does not list it"
            ),
        ));
    }
    Ok(Json(PresetEditBody {
        ok: true,
        action: "add".to_string(),
        id,
        name: label,
        presets,
        lines,
        session_ms,
        command_ms,
    }))
}

#[allow(clippy::too_many_arguments)]
#[post("/device/controls/preset/rename?<token>&<name>&<device>")]
async fn preset_rename(
    token: String,
    name: String,
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<PresetEditBody>, Failure> {
    let id = preset_id(&token)?;
    if name.trim().is_empty() {
        return Err(fail(Status::BadRequest, "a preset name cannot be blank"));
    }
    let label = preset_name(Some(&name), id);
    let control = dvrip_of(device.as_deref(), cfg, cloud, modes, store, cookies)?;
    let Opened {
        connection,
        session_ms,
    } = open(&control).await.map_err(control_failure)?;
    let mut connection = gate(&control, connection, &PRESET_ABILITY, "preset rename").await?;
    let outcome = async {
        let (_, before) = listing(&control, &mut connection, "preset listing").await?;
        if !listing_has(&before, id) {
            return Ok(Err(fail(
                Status::NotFound,
                format!("preset {id} is not listed under Uart.PTZPreset"),
            )));
        }
        let started = Instant::now();
        within(
            "preset rename",
            &control,
            op_window(),
            connection.rename_preset(id, &label),
        )
        .await?;
        let command_ms = ms(started.elapsed());
        let (lines, presets) = listing(&control, &mut connection, "preset read back").await?;
        Ok(Ok((lines, presets, command_ms)))
    }
    .await;
    let (lines, presets, command_ms) = finish(connection, outcome)
        .await
        .map_err(control_failure)??;
    Ok(Json(PresetEditBody {
        ok: true,
        action: "rename".to_string(),
        id,
        name: label,
        presets,
        lines,
        session_ms,
        command_ms,
    }))
}

#[allow(clippy::too_many_arguments)]
#[post("/device/controls/preset/delete?<token>&<device>")]
async fn preset_delete(
    token: String,
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<PresetEditBody>, Failure> {
    let id = preset_id(&token)?;
    let control = dvrip_of(device.as_deref(), cfg, cloud, modes, store, cookies)?;
    let Opened {
        connection,
        session_ms,
    } = open(&control).await.map_err(control_failure)?;
    let mut connection = gate(&control, connection, &PRESET_ABILITY, "preset delete").await?;
    let outcome = async {
        let (tours, before) = listing(&control, &mut connection, "preset listing").await?;
        let Some(entry) = before.iter().find(|preset| preset.id == id).cloned() else {
            return Ok(Err(fail(
                Status::NotFound,
                format!("preset {id} is not listed under Uart.PTZPreset"),
            )));
        };
        let started = Instant::now();
        within(
            "preset delete",
            &control,
            op_window(),
            connection.clear_preset(id),
        )
        .await?;
        for line in tours
            .iter()
            .filter(|line| line.points.iter().any(|point| point.preset == id))
        {
            within(
                "tour point delete",
                &control,
                op_window(),
                connection.delete_tour_point(line.channel, line.id, id),
            )
            .await?;
        }
        let command_ms = ms(started.elapsed());
        let (lines, presets) = listing(&control, &mut connection, "preset read back").await?;
        Ok(Ok((entry.name, lines, presets, command_ms)))
    }
    .await;
    let (label, lines, presets, command_ms) = finish(connection, outcome)
        .await
        .map_err(control_failure)??;
    if listing_has(&presets, id) {
        return Err(fail(
            Status::BadGateway,
            format!("preset delete: the camera acked ClearPreset {id} but Uart.PTZPreset still lists it"),
        ));
    }
    Ok(Json(PresetEditBody {
        ok: true,
        action: "delete".to_string(),
        id,
        name: label,
        presets,
        lines,
        session_ms,
        command_ms,
    }))
}

#[derive(Serialize)]
pub struct TourEditBody {
    pub ok: bool,
    pub tour: i64,
    pub preset: i64,
    pub lines: Vec<TourLine>,
    pub command_ms: u64,
}

#[allow(clippy::too_many_arguments)]
#[post("/device/controls/tour/remove?<tour>&<preset>&<device>")]
async fn tour_remove(
    tour: i64,
    preset: i64,
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<TourEditBody>, Failure> {
    let control = dvrip_of(device.as_deref(), cfg, cloud, modes, store, cookies)?;
    let Opened { connection, .. } = open(&control).await.map_err(control_failure)?;
    let mut connection = gate(&control, connection, &PRESET_ABILITY, "tour point delete").await?;
    let outcome = async {
        let (before, _) = listing(&control, &mut connection, "tour listing").await?;
        let Some(line) = before.iter().find(|line| line.id == tour) else {
            return Ok(Err(fail(
                Status::NotFound,
                format!("tour {tour} is not listed under Uart.PTZTour"),
            )));
        };
        if !line.points.iter().any(|point| point.preset == preset) {
            return Ok(Err(fail(
                Status::NotFound,
                format!("tour {tour} has no stop at preset {preset}"),
            )));
        }
        let started = Instant::now();
        within(
            "tour point delete",
            &control,
            op_window(),
            connection.delete_tour_point(line.channel, tour, preset),
        )
        .await?;
        let command_ms = ms(started.elapsed());
        let (lines, _) = listing(&control, &mut connection, "tour read back").await?;
        Ok(Ok((lines, command_ms)))
    }
    .await;
    let (lines, command_ms) = finish(connection, outcome)
        .await
        .map_err(control_failure)??;
    Ok(Json(TourEditBody {
        ok: true,
        tour,
        preset,
        lines,
        command_ms,
    }))
}

#[derive(Serialize)]
pub struct CalibrateBody {
    pub ok: bool,
    pub session_ms: u64,
    pub calibrate_ms: u64,
}

#[post("/device/controls/ptz/calibrate?<device>")]
async fn ptz_calibrate(
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<CalibrateBody>, Failure> {
    let control = dvrip_of(device.as_deref(), cfg, cloud, modes, store, cookies)?;
    let Opened {
        connection,
        session_ms,
    } = open(&control).await.map_err(control_failure)?;
    let mut connection = gate(&control, connection, &CALIBRATE_ABILITY, "ptz calibrate").await?;
    let started = Instant::now();
    let sent = within(
        "ptz calibrate",
        &control,
        CALIBRATE_WINDOW,
        connection.ptz_auto_adjust(),
    )
    .await;
    let calibrate_ms = ms(started.elapsed());
    finish(connection, sent).await.map_err(control_failure)?;
    Ok(Json(CalibrateBody {
        ok: true,
        session_ms,
        calibrate_ms,
    }))
}

#[derive(Serialize)]
pub struct PanoramaBody {
    pub ok: bool,
    pub range: PanoramaRange,
}

async fn read_range(control: &DvripControl) -> Result<PanoramaRange, Failure> {
    let Opened { mut connection, .. } = open(control).await.map_err(control_failure)?;
    let read = match tokio::time::timeout(op_window(), connection.panorama_range()).await {
        Ok(Err(DvrError::Rejected { ret, reason })) => {
            let _ = connection.logout_within(op_window()).await;
            return Err(fail(
                Status::NotImplemented,
                format!("the camera refused the GoToPosition ability with Ret {ret} ({reason}), so the app hides its panoramic PTZ screen"),
            ));
        }
        Ok(result) => {
            result.map_err(|error| ControlError::Device(format!("panorama range read: {error}")))
        }
        Err(_) => Err(timed_out("panorama range read", control, op_window())),
    };
    let range = finish(connection, read).await.map_err(control_failure)?;
    range.ok_or_else(|| {
        fail(
            Status::NotImplemented,
            "the camera answered GoToPosition without HorSteps, so the app hides its panoramic PTZ screen",
        )
    })
}

#[get("/device/controls/panorama?<device>")]
async fn panorama_read(
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<PanoramaBody>, Failure> {
    let control = dvrip_of(device.as_deref(), cfg, cloud, modes, store, cookies)?;
    let range = read_range(&control).await?;
    Ok(Json(PanoramaBody { ok: true, range }))
}

#[allow(clippy::too_many_arguments)]
#[post("/device/controls/panorama/goto?<left>&<device>")]
async fn panorama_goto(
    left: i64,
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<PanoramaBody>, Failure> {
    let control = dvrip_of(device.as_deref(), cfg, cloud, modes, store, cookies)?;
    let range = read_range(&control).await?;
    let end = range.hor_start + range.hor_steps;
    if left < range.hor_start || left > end {
        return Err(fail(
            Status::BadRequest,
            format!(
                "left must lie between HorStart {} and {end}",
                range.hor_start
            ),
        ));
    }
    let Opened { mut connection, .. } = open(&control).await.map_err(control_failure)?;
    let sent = within(
        "panorama goto",
        &control,
        op_window(),
        connection.goto_position(left, range.ver_start),
    )
    .await;
    finish(connection, sent).await.map_err(control_failure)?;
    Ok(Json(PanoramaBody { ok: true, range }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hold_limit_follows_its_env_knob() {
        std::env::remove_var("PROVIEW_PTZ_HOLD_MS");
        assert_eq!(hold_limit(), HOLD_LIMIT);
        std::env::set_var("PROVIEW_PTZ_HOLD_MS", "1500");
        assert_eq!(hold_limit(), Duration::from_millis(1500));
        std::env::set_var("PROVIEW_PTZ_HOLD_MS", "0");
        assert_eq!(hold_limit(), HOLD_LIMIT);
        std::env::remove_var("PROVIEW_PTZ_HOLD_MS");
    }
}
