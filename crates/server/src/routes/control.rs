use std::time::Duration;

use proview_core::config::Config;
use proview_core::device::{Backend, ControlError, Controls, DeviceControl, Lens};
use proview_core::dvrip::control::{RecordMode, WhiteLightMode};
use proview_core::session::Store;
use rocket::form::FromForm;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;

use super::{fail, session_id, Failure};

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

#[get("/device/controls")]
async fn capability_report(
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<CapabilityBody>, Failure> {
    signed_in(store, cookies)?;
    let controls = controls(cfg);
    let rows = controls.capability_report().await;
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
    outcome.map_err(control_failure)?;
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
