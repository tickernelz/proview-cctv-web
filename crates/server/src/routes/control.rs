use std::time::Duration;

use proview_core::config::Config;
use proview_core::device::{talk_capability, Backend, ControlError, Controls, DeviceControl, Lens};
use proview_core::dvrip::control::{PresetEntry, PtzLocate, TourLine};
use proview_core::dvrip::control::{RecordMode, WhiteLightMode};
use proview_core::session::Store;
use rocket::form::FromForm;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;

use super::{fail, fail_code, restore, session_id, Failure};

pub fn routes() -> Vec<rocket::route::Route> {
    routes![
        capability_report,
        ptz_move,
        ptz_stop,
        ptz_presets,
        ptz_goto_preset,
        light_read,
        light_write,
        flip_read,
        flip_write,
        record_read,
        record_write,
        snapshot,
        settings,
        tour_state,
        tour_start,
        tour_stop,
        tour_add,
        tour_clear,
    ]
}

fn controls(cfg: &State<Config>) -> Controls {
    Controls::from_config(cfg)
}

fn signed_in(store: &State<Store>, cookies: &CookieJar<'_>) -> Result<(), Failure> {
    let id = session_id(cookies)?;
    store
        .get(&id)
        .map(|_| ())
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))
}

fn control_status(error: &ControlError) -> Status {
    match error {
        ControlError::Unsupported { .. } => Status::NotImplemented,
        ControlError::NotConfigured(_) => Status::BadRequest,
        ControlError::Blocked { .. } => Status::Conflict,
        ControlError::Timeout { .. } => Status::GatewayTimeout,
        ControlError::Device(_) => Status::BadGateway,
    }
}

fn control_failure(error: ControlError) -> Failure {
    fail(control_status(&error), error.to_string())
}

fn lens_name(lens: Lens) -> String {
    match lens {
        Lens::Top => "top".to_string(),
        Lens::Bottom => "bottom".to_string(),
        Lens::Device => "device".to_string(),
    }
}

fn parse_lens(raw: Option<&str>) -> Result<Lens, Failure> {
    match raw.unwrap_or("bottom") {
        "bottom" => Ok(Lens::Bottom),
        "top" => Ok(Lens::Top),
        other => Err(fail(
            Status::BadRequest,
            format!("unknown lens {other}; use bottom or top"),
        )),
    }
}

#[derive(Serialize)]
pub struct CapabilityRow {
    pub lens: String,
    pub feature: String,
    pub state: String,
    pub backend: String,
    pub evidence: String,
}

#[derive(Serialize)]
pub struct CapabilityBody {
    pub ok: bool,
    pub onvif: bool,
    pub dvrip: bool,
    pub capabilities: Vec<CapabilityRow>,
}

#[get("/device/controls?<device>")]
async fn capability_report(
    device: Option<String>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<CapabilityBody>, Failure> {
    signed_in(store, cookies)?;
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    let controls = controls(cfg);
    let mut rows = controls.capability_report().await;
    rows.push(talk_capability(
        cfg,
        vendor.token.as_deref(),
        device.as_deref(),
    ));
    Ok(Json(CapabilityBody {
        ok: true,
        onvif: controls.onvif.is_some(),
        dvrip: controls.dvrip.is_some(),
        capabilities: rows
            .into_iter()
            .map(|row| CapabilityRow {
                lens: lens_name(row.lens),
                feature: row.feature,
                state: match row.state {
                    proview_core::device::State::Supported => "supported".to_string(),
                    proview_core::device::State::Blocked => "blocked".to_string(),
                },
                backend: row.backend.as_str().to_string(),
                evidence: row.evidence,
            })
            .collect(),
    }))
}

#[derive(Serialize)]
pub struct MoveBody {
    pub ok: bool,
    pub lens: String,
    pub backend: String,
    pub pan: f32,
    pub tilt: f32,
    pub zoom: f32,
    pub hold_ms: u64,
}

#[derive(FromForm)]
pub struct PtzQuery {
    lens: Option<String>,
    pan: Option<f32>,
    tilt: Option<f32>,
    zoom: Option<f32>,
    hold_ms: Option<u64>,
}

#[post("/device/controls/ptz?<query..>")]
async fn ptz_move(
    query: PtzQuery,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<MoveBody>, Failure> {
    signed_in(store, cookies)?;
    let lens = parse_lens(query.lens.as_deref())?;
    let pan = query.pan.unwrap_or(0.0).clamp(-1.0, 1.0);
    let tilt = query.tilt.unwrap_or(0.0).clamp(-1.0, 1.0);
    let zoom = query.zoom.unwrap_or(0.0).clamp(-1.0, 1.0);
    let hold = Duration::from_millis(query.hold_ms.unwrap_or(600).clamp(100, 5000));
    if pan == 0.0 && tilt == 0.0 && zoom == 0.0 {
        return Err(fail(Status::BadRequest, "a move needs a nonzero velocity"));
    }
    let controls = controls(cfg);
    let (backend, outcome) = match controls.onvif.as_ref() {
        Some(control) => (
            Backend::Onvif,
            control.move_ptz(lens, pan, tilt, zoom, hold).await,
        ),
        None => {
            let control = controls.dvrip().map_err(control_failure)?;
            (
                Backend::Dvrip,
                control.move_ptz(lens, pan, tilt, zoom, hold).await,
            )
        }
    };
    if let Err(error) = outcome {
        return Err(match error {
            ControlError::Blocked { operation, reason } if lens == Lens::Top => {
                proview_core::device::refresh_ptz_binding(cfg);
                let evidence = match proview_core::device::cached_ptz_binding(cfg) {
                    Some(binding) => format!("{reason}; {}", binding.evidence),
                    None => reason.to_string(),
                };
                fail_code(
                    Status::Conflict,
                    "ptz_lens_has_no_motor",
                    format!("{operation}: {evidence}; the motor answers for lens=bottom"),
                )
            }
            other => control_failure(other),
        });
    }
    Ok(Json(MoveBody {
        ok: true,
        lens: lens_name(lens),
        backend: backend.as_str().to_string(),
        pan,
        tilt,
        zoom,
        hold_ms: hold.as_millis() as u64,
    }))
}

#[derive(Serialize)]
pub struct SimpleBody {
    pub ok: bool,
    pub detail: String,
}

#[post("/device/controls/ptz/stop")]
async fn ptz_stop(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<SimpleBody>, Failure> {
    signed_in(store, cookies)?;
    let controls = controls(cfg);
    let control = controls.onvif().map_err(control_failure)?;
    control.stop_ptz().await.map_err(control_failure)?;
    Ok(Json(SimpleBody {
        ok: true,
        detail: "pan/tilt/zoom stopped".to_string(),
    }))
}

#[derive(Serialize)]
pub struct PresetBody {
    pub token: String,
    pub name: String,
}

#[get("/device/controls/presets")]
async fn ptz_presets(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<Vec<PresetBody>>, Failure> {
    signed_in(store, cookies)?;
    let controls = controls(cfg);
    let control = controls.onvif().map_err(control_failure)?;
    let presets = control.presets().await.map_err(control_failure)?;
    Ok(Json(
        presets
            .into_iter()
            .map(|preset| PresetBody {
                token: preset.token,
                name: preset.name,
            })
            .collect(),
    ))
}

#[post("/device/controls/preset/goto?<token>")]
async fn ptz_goto_preset(
    token: String,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<PresetBody>, Failure> {
    signed_in(store, cookies)?;
    let controls = controls(cfg);
    let control = controls.onvif().map_err(control_failure)?;
    control.goto_preset(&token).await.map_err(control_failure)?;
    Ok(Json(PresetBody {
        token,
        name: String::new(),
    }))
}

#[derive(Serialize)]
pub struct LightBody {
    pub ok: bool,
    pub mode: Option<String>,
}

#[get("/device/controls/light")]
async fn light_read(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<LightBody>, Failure> {
    signed_in(store, cookies)?;
    let controls = controls(cfg);
    let control = controls.dvrip().map_err(control_failure)?;
    let mode = control.white_light(None).await.map_err(control_failure)?;
    Ok(Json(LightBody { ok: true, mode }))
}

#[post("/device/controls/light?<mode>")]
async fn light_write(
    mode: String,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<LightBody>, Failure> {
    signed_in(store, cookies)?;
    let parsed = WhiteLightMode::parse(&mode).ok_or_else(|| {
        fail(
            Status::BadRequest,
            "mode must be off, intelligent or always",
        )
    })?;
    let controls = controls(cfg);
    let control = controls.dvrip().map_err(control_failure)?;
    let applied = control
        .white_light(Some(parsed))
        .await
        .map_err(control_failure)?;
    Ok(Json(LightBody {
        ok: true,
        mode: applied,
    }))
}

#[derive(Serialize)]
pub struct FlipBody {
    pub ok: bool,
    pub flip: Option<String>,
    pub mirror: Option<String>,
}

#[get("/device/controls/flip")]
async fn flip_read(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<FlipBody>, Failure> {
    signed_in(store, cookies)?;
    let controls = controls(cfg);
    let control = controls.dvrip().map_err(control_failure)?;
    let (flip, mirror) = control
        .picture_flip(None, None)
        .await
        .map_err(control_failure)?;
    Ok(Json(FlipBody {
        ok: true,
        flip,
        mirror,
    }))
}

#[post("/device/controls/flip?<flip>&<mirror>")]
async fn flip_write(
    flip: Option<bool>,
    mirror: Option<bool>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<FlipBody>, Failure> {
    signed_in(store, cookies)?;
    if flip.is_none() && mirror.is_none() {
        return Err(fail(
            Status::BadRequest,
            "set flip and/or mirror to true or false",
        ));
    }
    let controls = controls(cfg);
    let control = controls.dvrip().map_err(control_failure)?;
    let (flip, mirror) = control
        .picture_flip(flip, mirror)
        .await
        .map_err(control_failure)?;
    Ok(Json(FlipBody {
        ok: true,
        flip,
        mirror,
    }))
}

#[derive(Serialize)]
pub struct RecordBody {
    pub ok: bool,
    pub mode: Option<String>,
}

#[get("/device/controls/record")]
async fn record_read(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<RecordBody>, Failure> {
    signed_in(store, cookies)?;
    let controls = controls(cfg);
    let control = controls.dvrip().map_err(control_failure)?;
    let mode = control.record_mode(None).await.map_err(control_failure)?;
    Ok(Json(RecordBody { ok: true, mode }))
}

#[post("/device/controls/record?<mode>")]
async fn record_write(
    mode: String,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<RecordBody>, Failure> {
    signed_in(store, cookies)?;
    let parsed = RecordMode::parse(&mode)
        .ok_or_else(|| fail(Status::BadRequest, "mode must be config, manual or off"))?;
    let controls = controls(cfg);
    let control = controls.dvrip().map_err(control_failure)?;
    let applied = control
        .record_mode(Some(parsed))
        .await
        .map_err(control_failure)?;
    Ok(Json(RecordBody {
        ok: true,
        mode: applied,
    }))
}

#[get("/device/controls/snapshot")]
async fn snapshot(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<(rocket::http::ContentType, Vec<u8>), Failure> {
    signed_in(store, cookies)?;
    let controls = controls(cfg);
    let control = controls.dvrip().map_err(control_failure)?;
    let picture = control.snapshot().await.map_err(control_failure)?;
    Ok((rocket::http::ContentType::JPEG, picture))
}

#[derive(Serialize)]
pub struct TourStateBody {
    pub ok: bool,
    pub backend: String,
    pub lines: Vec<TourLine>,
    pub presets: Vec<PresetEntry>,
    pub locate: Option<PtzLocate>,
    pub note: String,
}

const TOUR_LISTING_NOTE: &str = "this listing is the device's own configuration; the device answers every OPPTZControl with Ret 100 before it moves, so only the frames prove motion";

const TOUR_ACTION_NOTE: &str = "Ret 100 is the device's ack, sent before the motor moves; only frames prove motion, and GET /api/device/controls/tour shows what the device stored";

#[get("/device/controls/tour")]
async fn tour_state(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<TourStateBody>, Failure> {
    signed_in(store, cookies)?;
    let controls = controls(cfg);
    let control = controls.dvrip().map_err(control_failure)?;
    let state = control.tour_state().await.map_err(control_failure)?;
    Ok(Json(TourStateBody {
        ok: true,
        backend: Backend::Dvrip.as_str().to_string(),
        lines: state.lines,
        presets: state.presets,
        locate: state.locate,
        note: TOUR_LISTING_NOTE.to_string(),
    }))
}

#[derive(Serialize)]
pub struct TourActionBody {
    pub ok: bool,
    pub backend: String,
    pub action: String,
    pub tour: i64,
    pub index: Option<i64>,
    pub note: String,
}

fn tour_action_body(action: &str, tour: i64, index: Option<i64>) -> TourActionBody {
    TourActionBody {
        ok: true,
        backend: Backend::Dvrip.as_str().to_string(),
        action: action.to_string(),
        tour,
        index,
        note: TOUR_ACTION_NOTE.to_string(),
    }
}

#[post("/device/controls/tour/start?<tour>")]
async fn tour_start(
    tour: i64,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<TourActionBody>, Failure> {
    signed_in(store, cookies)?;
    let controls = controls(cfg);
    let control = controls.dvrip().map_err(control_failure)?;
    control.start_tour(tour).await.map_err(control_failure)?;
    Ok(Json(tour_action_body("start", tour, None)))
}

#[post("/device/controls/tour/stop?<tour>")]
async fn tour_stop(
    tour: i64,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<TourActionBody>, Failure> {
    signed_in(store, cookies)?;
    let controls = controls(cfg);
    let control = controls.dvrip().map_err(control_failure)?;
    control.stop_tour(tour).await.map_err(control_failure)?;
    Ok(Json(tour_action_body("stop", tour, None)))
}

#[post("/device/controls/tour/add?<tour>&<preset>&<index>")]
async fn tour_add(
    tour: i64,
    preset: i64,
    index: Option<i64>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<TourActionBody>, Failure> {
    signed_in(store, cookies)?;
    if preset <= 0 {
        return Err(fail(
            Status::BadRequest,
            "preset must be the device preset id reported under Uart.PTZPreset",
        ));
    }
    let index = index.unwrap_or(0).max(0);
    let controls = controls(cfg);
    let control = controls.dvrip().map_err(control_failure)?;
    control
        .add_tour_point(tour, preset, index)
        .await
        .map_err(control_failure)?;
    Ok(Json(tour_action_body("add", tour, Some(index))))
}

#[post("/device/controls/tour/clear?<tour>")]
async fn tour_clear(
    tour: i64,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<TourActionBody>, Failure> {
    signed_in(store, cookies)?;
    let controls = controls(cfg);
    let control = controls.dvrip().map_err(control_failure)?;
    control.clear_tour(tour).await.map_err(control_failure)?;
    Ok(Json(tour_action_body("clear", tour, None)))
}

#[get("/device/controls/settings")]
async fn settings(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<serde_json::Value>, Failure> {
    signed_in(store, cookies)?;
    let controls = controls(cfg);
    let control = controls.onvif().map_err(control_failure)?;
    let settings = control.settings().await.map_err(control_failure)?;
    Ok(Json(settings))
}
