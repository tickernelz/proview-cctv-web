use std::sync::Arc;
use std::time::Duration;

use proview_core::config::Config;
use proview_core::device::{ControlError, SessionError};
use proview_core::dvrip::client::Connection;
use proview_core::dvrip::image::{
    self, Edit, ImageError, ImageReport, LightPatch, SleepPatch, VolumeTarget, WriteOutcome,
};
use proview_core::provider::cloud::CloudProvider;
use proview_core::session::Store;
use rocket::form::FromForm;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;

use super::control::controls_for;
use super::{fail, Failure};
use crate::config::ModeStore;

const WINDOW_ENV: &str = "PROVIEW_SETTINGS_WINDOW_MS";
const DEFAULT_WINDOW: Duration = Duration::from_secs(20);

pub fn routes() -> Vec<rocket::route::Route> {
    routes![
        report, live, day_night, scene, defog, low_lux, light, privacy, sleep, indicators, volume
    ]
}

fn window() -> Duration {
    match std::env::var(WINDOW_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
    {
        Some(ms) if ms > 0 => Duration::from_millis(ms),
        _ => DEFAULT_WINDOW,
    }
}

fn image_failure(error: ImageError) -> Failure {
    let status = match &error {
        ImageError::Timeout { .. } => Status::GatewayTimeout,
        ImageError::Unsupported { .. } => Status::NotImplemented,
        ImageError::Invalid(_) => Status::BadRequest,
        ImageError::Device { .. } => Status::BadGateway,
    };
    fail(status, error.to_string())
}

fn session_failure(error: SessionError) -> Failure {
    match error {
        SessionError::Timeout { window, .. } => fail(
            Status::GatewayTimeout,
            format!(
                "connect and login did not answer inside the image settings window ({window:?})"
            ),
        ),
        SessionError::Failed(error) => {
            let status = match &error {
                ControlError::Timeout { .. } => Status::GatewayTimeout,
                ControlError::NotConfigured(_) => Status::BadRequest,
                ControlError::Unsupported { .. } => Status::NotImplemented,
                ControlError::Blocked { .. } => Status::Conflict,
                ControlError::Device(_) => Status::BadGateway,
            };
            fail(status, error.to_string())
        }
    }
}

struct Gear<'a> {
    cfg: &'a Config,
    cloud: &'a Arc<CloudProvider>,
    modes: &'a ModeStore,
    store: &'a Store,
    cookies: &'a CookieJar<'a>,
}

async fn open(
    device: Option<&str>,
    gear: &Gear<'_>,
    window: Duration,
) -> Result<Connection, Failure> {
    let controls = controls_for(
        device,
        gear.cfg,
        gear.cloud,
        gear.modes,
        gear.store,
        gear.cookies,
    )?;
    let control = controls
        .dvrip()
        .map_err(|error| session_failure(SessionError::Failed(error)))?;
    control
        .session_within(window)
        .await
        .map_err(session_failure)
}

#[derive(Serialize)]
pub struct ReportBody {
    pub ok: bool,
    #[serde(flatten)]
    pub report: ImageReport,
}

#[get("/device/image?<device>")]
async fn report(
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<ReportBody>, Failure> {
    let gear = Gear {
        cfg,
        cloud,
        modes,
        store,
        cookies,
    };
    let window = window();
    let mut connection = open(device.as_deref(), &gear, window).await?;
    let result = image::read_report(&mut connection, window).await;
    let _ = connection.logout_within(window).await;
    Ok(Json(ReportBody {
        ok: true,
        report: result.map_err(image_failure)?,
    }))
}

#[derive(Serialize)]
pub struct LiveBody {
    pub ok: bool,
    #[serde(flatten)]
    pub live: image::LiveState,
}

#[get("/device/image/live?<device>")]
async fn live(
    device: Option<String>,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<LiveBody>, Failure> {
    let gear = Gear {
        cfg,
        cloud,
        modes,
        store,
        cookies,
    };
    let window = window();
    let mut connection = open(device.as_deref(), &gear, window).await?;
    let result = image::read_live(&mut connection, window).await;
    let _ = connection.logout_within(window).await;
    Ok(Json(LiveBody {
        ok: true,
        live: result.map_err(image_failure)?,
    }))
}

#[derive(Serialize)]
pub struct WriteBody {
    pub ok: bool,
    pub feature: &'static str,
    #[serde(flatten)]
    pub outcome: WriteOutcome,
}

async fn write<F>(
    device: Option<&str>,
    gear: Gear<'_>,
    edit: Edit<'_>,
    mutate: F,
) -> Result<(Status, Json<WriteBody>), Failure>
where
    F: FnOnce(&mut serde_json::Value) -> Result<(), ImageError>,
{
    let feature = edit.feature;
    let window = window();
    let mut connection = open(device, &gear, window).await?;
    let result = image::edit(&mut connection, window, edit, mutate).await;
    if !matches!(result, Err(ImageError::Timeout { .. })) {
        let _ = connection.logout_within(window).await;
    }
    let outcome = result.map_err(image_failure)?;
    let status = if outcome.matches {
        Status::Ok
    } else {
        Status::Conflict
    };
    Ok((
        status,
        Json(WriteBody {
            ok: outcome.matches,
            feature,
            outcome,
        }),
    ))
}

type Written = Result<(Status, Json<WriteBody>), Failure>;

#[derive(FromForm)]
pub struct DayNightQuery {
    code: i64,
    device: Option<String>,
}

#[post("/device/image/day-night?<query..>")]
async fn day_night(
    query: DayNightQuery,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Written {
    let gear = Gear {
        cfg,
        cloud,
        modes,
        store,
        cookies,
    };
    let edit = Edit {
        feature: "day/night mode",
        gates: &[],
        names: &[image::CAMERA_PARAM],
    };
    write(query.device.as_deref(), gear, edit, |entry| {
        image::apply_day_night(entry, query.code)
    })
    .await
}

#[derive(FromForm)]
pub struct SceneQuery {
    scene: i64,
    device: Option<String>,
}

#[post("/device/image/scene?<query..>")]
async fn scene(
    query: SceneQuery,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Written {
    let gear = Gear {
        cfg,
        cloud,
        modes,
        store,
        cookies,
    };
    let edit = Edit {
        feature: "scene",
        gates: &[],
        names: &[image::FISH_EYE],
    };
    write(query.device.as_deref(), gear, edit, |entry| {
        image::apply_scene(entry, query.scene)
    })
    .await
}

#[derive(FromForm)]
pub struct DefogQuery {
    enable: Option<bool>,
    level: Option<i64>,
    device: Option<String>,
}

#[post("/device/image/defog?<query..>")]
async fn defog(
    query: DefogQuery,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Written {
    let gear = Gear {
        cfg,
        cloud,
        modes,
        store,
        cookies,
    };
    let edit = Edit {
        feature: "defog",
        gates: &[],
        names: &[image::CLEAR_FOG],
    };
    write(query.device.as_deref(), gear, edit, |entry| {
        image::apply_defog(entry, query.enable, query.level)
    })
    .await
}

#[derive(FromForm)]
pub struct ToggleQuery {
    enable: bool,
    device: Option<String>,
}

#[post("/device/image/low-lux?<query..>")]
async fn low_lux(
    query: ToggleQuery,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Written {
    let gear = Gear {
        cfg,
        cloud,
        modes,
        store,
        cookies,
    };
    let edit = Edit {
        feature: "low-lux mode",
        gates: &["SupportLowLuxMode"],
        names: &[image::CAMERA_PARAM_EX],
    };
    write(query.device.as_deref(), gear, edit, |entry| {
        image::apply_low_lux(entry, query.enable)
    })
    .await
}

#[derive(FromForm)]
pub struct LightQuery {
    period: Option<bool>,
    start: Option<String>,
    end: Option<String>,
    level: Option<i64>,
    duration: Option<i64>,
    device: Option<String>,
}

#[post("/device/image/light?<query..>")]
async fn light(
    query: LightQuery,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Written {
    let gear = Gear {
        cfg,
        cloud,
        modes,
        store,
        cookies,
    };
    let patch = LightPatch {
        period: query.period,
        start: query.start.clone(),
        end: query.end.clone(),
        level: query.level,
        duration: query.duration,
    };
    let edit = Edit {
        feature: "white light schedule",
        gates: &["SupportCameraWhiteLight"],
        names: &[image::WHITE_LIGHT],
    };
    write(query.device.as_deref(), gear, edit, |entry| {
        image::apply_light(entry, &patch)
    })
    .await
}

#[post("/device/image/privacy?<query..>")]
async fn privacy(
    query: ToggleQuery,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Written {
    let gear = Gear {
        cfg,
        cloud,
        modes,
        store,
        cookies,
    };
    let edit = Edit {
        feature: "privacy shading",
        gates: &["SupportOneKeyMaskVideo"],
        names: &[image::MASK_VIDEO],
    };
    write(query.device.as_deref(), gear, edit, |entry| {
        image::apply_privacy(entry, query.enable)
    })
    .await
}

#[derive(FromForm)]
pub struct SleepQuery {
    enable: Option<bool>,
    start: Option<String>,
    end: Option<String>,
    repeat: Option<bool>,
    wake: Option<bool>,
    device: Option<String>,
}

#[post("/device/image/sleep?<query..>")]
async fn sleep(
    query: SleepQuery,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Written {
    let gear = Gear {
        cfg,
        cloud,
        modes,
        store,
        cookies,
    };
    let patch = SleepPatch {
        enable: query.enable,
        start: query.start.clone(),
        end: query.end.clone(),
        repeat_daily: query.repeat,
        wake: query.wake.unwrap_or(false),
    };
    let edit = Edit {
        feature: "timed sleep",
        gates: &["SupportTimingSleep"],
        names: &[image::TIMING_SLEEP],
    };
    write(query.device.as_deref(), gear, edit, |entry| {
        image::apply_sleep(entry, &patch)
    })
    .await
}

#[derive(FromForm)]
pub struct IndicatorQuery {
    led: Option<bool>,
    voice_tip: Option<bool>,
    device: Option<String>,
}

#[post("/device/image/indicators?<query..>")]
async fn indicators(
    query: IndicatorQuery,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Written {
    let gear = Gear {
        cfg,
        cloud,
        modes,
        store,
        cookies,
    };
    let gates: &[&'static str] = match (query.led.is_some(), query.voice_tip.is_some()) {
        (true, true) => &["SupportStatusLed", "SupportCloseVoiceTip"],
        (true, false) => &["SupportStatusLed"],
        (false, true) => &["SupportCloseVoiceTip"],
        (false, false) => {
            return Err(fail(Status::BadRequest, "name led and/or voice_tip"));
        }
    };
    let edit = Edit {
        feature: if query.led.is_some() {
            "status LED"
        } else {
            "voice tip"
        },
        gates,
        names: &[image::EXTRA_STATE],
    };
    write(query.device.as_deref(), gear, edit, |entry| {
        image::apply_indicators(entry, query.led, query.voice_tip)
    })
    .await
}

#[derive(FromForm)]
pub struct VolumeQuery {
    target: String,
    left: Option<i64>,
    right: Option<i64>,
    level: Option<i64>,
    device: Option<String>,
}

#[post("/device/image/volume?<query..>")]
async fn volume(
    query: VolumeQuery,
    cfg: &State<Config>,
    cloud: &State<Arc<CloudProvider>>,
    modes: &State<ModeStore>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Written {
    let gear = Gear {
        cfg,
        cloud,
        modes,
        store,
        cookies,
    };
    let target = VolumeTarget::parse(&query.target)
        .ok_or_else(|| fail(Status::BadRequest, "target must be speaker or mic"))?;
    let left = query.left.or(query.level);
    let right = query.right.or(query.level);
    let edit = match target {
        VolumeTarget::Speaker => Edit {
            feature: "speaker volume",
            gates: &["SupportSetVolume"],
            names: &[image::SPEAKER_VOLUME],
        },
        VolumeTarget::Mic => Edit {
            feature: "microphone volume",
            gates: &["SupportSetVolume"],
            names: &[image::MIC_VOLUME, image::MIC_VOLUME_ALIAS],
        },
    };
    write(query.device.as_deref(), gear, edit, |entry| {
        image::apply_volume(entry, left, right)
    })
    .await
}
