#[path = "routes/alarms.rs"]
pub mod alarms;

#[path = "routes/apmode.rs"]
pub mod apmode;

#[path = "routes/cloud.rs"]
pub mod cloud;

#[path = "routes/grid.rs"]
pub mod grid;

#[path = "routes/cloud_bind.rs"]
pub mod cloud_bind;

use std::time::Duration;

use proview_core::config::Config;
use proview_core::device::ControlError;
use proview_core::live::{LiveMode, StreamKey};
use proview_core::provider::{
    self, CloudPath, Device, Provider, ProviderError, Router, StreamTicket,
};
use proview_core::session::{self, Store};
use rocket::http::{ContentType, Cookie, CookieJar, SameSite, Status};
use rocket::response::content::{RawCss, RawHtml, RawJavaScript};
use rocket::serde::json::Json;
use rocket::State;
use rocket_ws as ws;
use serde::{Deserialize, Serialize};

use crate::config::ModeStore;

pub mod audio;
pub mod control;

pub fn files() -> Vec<rocket::route::Route> {
    routes![index, app_js, account_js, talk_js, grid_js, style_css, tokens_css, font, media_clip]
        .into_iter()
        .chain(alarms::files())
        .collect()
}

pub fn api() -> Vec<rocket::route::Route> {
    routes![
        login,
        logout,
        me,
        devices,
        stream,
        cloud_path,
        probe_device,
        rtsp_probe,
        health,
        crate::stream::stats
    ]
    .into_iter()
    .chain(control::routes())
    .chain(alarms::routes())
    .chain(grid::routes())
    .chain(cloud_bind::api())
    .chain(apmode::routes())
    .collect()
}

pub fn live() -> Vec<rocket::route::Route> {
    routes![ws_live]
        .into_iter()
        .chain(audio::routes())
        .collect()
}

#[get("/")]
fn index() -> RawHtml<&'static str> {
    RawHtml(include_str!("../../../web/index.html"))
}

#[get("/app.js")]
fn app_js() -> RawJavaScript<&'static str> {
    RawJavaScript(include_str!("../../../web/app.js"))
}

#[get("/grid.js")]
fn grid_js() -> RawJavaScript<&'static str> {
    RawJavaScript(include_str!("../../../web/grid.js"))
}

#[get("/account.js")]
fn account_js() -> RawJavaScript<&'static str> {
    RawJavaScript(include_str!("../../../web/account.js"))
}

#[get("/talk.js")]
fn talk_js() -> RawJavaScript<&'static str> {
    RawJavaScript(include_str!("../../../web/talk.js"))
}

#[get("/tokens.css")]
fn tokens_css() -> RawCss<&'static str> {
    RawCss(include_str!("../../../web/tokens.css"))
}

#[get("/style.css")]
fn style_css() -> RawCss<&'static str> {
    RawCss(include_str!("../../../web/style.css"))
}

#[get("/fonts/<name>")]
fn font(name: &str) -> Option<(ContentType, &'static [u8])> {
    let woff2 = ContentType::new("font", "woff2");
    match name {
        "plex-sans-400.woff2" => Some((
            woff2,
            include_bytes!("../../../web/fonts/plex-sans-400.woff2"),
        )),
        "plex-sans-500.woff2" => Some((
            woff2,
            include_bytes!("../../../web/fonts/plex-sans-500.woff2"),
        )),
        "plex-sans-600.woff2" => Some((
            woff2,
            include_bytes!("../../../web/fonts/plex-sans-600.woff2"),
        )),
        "plex-mono-400.woff2" => Some((
            woff2,
            include_bytes!("../../../web/fonts/plex-mono-400.woff2"),
        )),
        "plex-mono-500.woff2" => Some((
            woff2,
            include_bytes!("../../../web/fonts/plex-mono-500.woff2"),
        )),
        "OFL.txt" => Some((
            ContentType::Plain,
            include_bytes!("../../../web/fonts/OFL.txt"),
        )),
        _ => None,
    }
}

#[get("/health")]
fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "service": "proview-server" }))
}

#[derive(Serialize)]
pub struct StatusBody {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
}

pub(crate) type Failure = (Status, Json<StatusBody>);

pub(crate) fn fail(status: Status, message: impl Into<String>) -> Failure {
    (
        status,
        Json(StatusBody {
            ok: false,
            message: message.into(),
            code: None,
        }),
    )
}

pub(crate) fn fail_code(status: Status, code: &'static str, message: impl Into<String>) -> Failure {
    (
        status,
        Json(StatusBody {
            ok: false,
            message: message.into(),
            code: Some(code),
        }),
    )
}

fn status_for(err: &ProviderError) -> Status {
    match err {
        ProviderError::BadCredentials | ProviderError::Expired => Status::Unauthorized,
        ProviderError::UnknownDevice(_) => Status::NotFound,
        ProviderError::NotImplemented(_) => Status::NotImplemented,
        ProviderError::Misconfigured(_) => Status::NotImplemented,
        _ => Status::BadGateway,
    }
}

pub(crate) fn signed_in(store: &State<Store>, cookies: &CookieJar<'_>) -> Result<(), Failure> {
    let id = session_id(cookies)?;
    store
        .get(&id)
        .map(|_| ())
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))
}

pub(crate) fn session_id(cookies: &CookieJar<'_>) -> Result<String, Failure> {
    cookies
        .get_private("pvw_session")
        .map(|c| c.value().to_string())
        .ok_or_else(|| fail(Status::Unauthorized, "not signed in"))
}

pub(crate) fn restore(store: &State<Store>, id: &str) -> Result<provider::VendorSession, Failure> {
    store
        .get(id)
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))
}

#[derive(Deserialize)]
pub struct LoginBody {
    email: String,
    password: String,
}

#[post("/login", data = "<body>")]
async fn login(
    body: Json<LoginBody>,
    cfg: &State<Config>,
    store: &State<Store>,
    modes: &State<ModeStore>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StatusBody>, Failure> {
    let current = cookies
        .get_private("pvw_session")
        .map(|c| c.value().to_string());
    let router = Router::new(cfg, modes.mode_for(current.as_deref()));
    let vendor = router
        .login(&body.email, &body.password)
        .await
        .map_err(|e| fail(status_for(&e), e.to_string()))?;

    let id = session::new_id();
    store.put(
        id.clone(),
        vendor,
        Duration::from_secs(cfg.session_ttl_secs),
    );
    cookies.add_private(
        Cookie::build(("pvw_session", id))
            .http_only(true)
            .same_site(SameSite::Strict)
            .build(),
    );
    let message = match router.resolution().notice {
        Some(notice) => format!("signed in via {}: {}", notice.to.as_str(), notice.message),
        None => format!("signed in via {}", router.source().as_str()),
    };
    Ok(Json(StatusBody {
        ok: true,
        message,
        code: None,
    }))
}

#[post("/logout")]
fn logout(
    store: &State<Store>,
    modes: &State<ModeStore>,
    cloud: &State<crate::routes::cloud::CloudState>,
    cookies: &CookieJar<'_>,
) -> Json<StatusBody> {
    if let Some(c) = cookies.get_private("pvw_session") {
        store.remove(c.value());
        modes.forget(c.value());
        cloud.forget_devices(c.value());
        cookies.remove_private(c);
    }
    Json(StatusBody {
        ok: true,
        message: "signed out".to_string(),
        code: None,
    })
}

#[get("/me")]
fn me(store: &State<Store>, cookies: &CookieJar<'_>) -> Result<Json<serde_json::Value>, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    Ok(Json(serde_json::json!({ "account": vendor.account })))
}

#[derive(Serialize)]
pub struct RtspReport {
    pub ok: bool,
    pub host: String,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub framerate: Option<u32>,
}

const RTSP_CHANNEL: u32 = 1;

fn rtsp_target_stream(cfg: &Config, sub_stream: bool) -> Option<proview_core::source::RtspTarget> {
    let target = cfg.device_rtsp.clone()?;
    let (host, port) = target.rsplit_once(':').unwrap_or((target.as_str(), "554"));
    let port = port.parse().unwrap_or(554);
    let path = format!(
        "/user=&password=&channel={RTSP_CHANNEL}&stream={}.sdp?real_stream",
        u8::from(sub_stream)
    );
    Some(
        proview_core::source::RtspTarget::from_host_port(host, port)
            .with_path(&path)
            .with_credentials(&cfg.device_user, &cfg.device_pass),
    )
}

fn stream_key(
    device: &str,
    channel: u32,
    sub_stream: bool,
    mode: LiveMode,
    source: Option<&proview_core::source::RtspTarget>,
) -> StreamKey {
    match source {
        Some(_) => StreamKey::new("local", RTSP_CHANNEL, sub_stream, mode),
        None => StreamKey::new(device, channel, sub_stream, mode),
    }
}

#[post("/device/rtsp")]
async fn rtsp_probe(cfg: &State<Config>) -> Result<Json<RtspReport>, Failure> {
    let target = cfg.device_rtsp.clone().ok_or_else(|| {
        fail_code(
            Status::BadRequest,
            "device_rtsp_missing",
            ControlError::NotConfigured("PROVIEW_DEVICE_RTSP").to_string(),
        )
    })?;
    let (host, port) = target.rsplit_once(':').unwrap_or((target.as_str(), "554"));
    let port: u16 = port.parse().unwrap_or(554);
    let info = proview_core::rtsp::describe(host, port, "/", &cfg.device_user, &cfg.device_pass)
        .await
        .map_err(|e| fail(Status::BadGateway, format!("rtsp {target}: {e}")))?;
    Ok(Json(RtspReport {
        ok: true,
        host: target,
        video_codec: info.video_codec,
        audio_codec: info.audio_codec,
        framerate: info.framerate,
    }))
}

#[get("/devices")]
async fn devices(
    cfg: &State<Config>,
    store: &State<Store>,
    modes: &State<ModeStore>,
    cookies: &CookieJar<'_>,
) -> Result<Json<Vec<Device>>, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    let router = Router::new(cfg, modes.mode_for(Some(&id)));
    router
        .devices(&vendor)
        .await
        .map(Json)
        .map_err(|e| fail(status_for(&e), e.to_string()))
}

#[derive(Serialize)]
pub struct DeviceProbe {
    pub ok: bool,
    pub host: String,
    pub session: u32,
    pub device_type: Option<String>,
    pub channels: Option<u32>,
    pub alive_interval: Option<u32>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub framerate: Option<u32>,
}

#[post("/device/probe")]
async fn probe_device(cfg: &State<Config>) -> Result<Json<DeviceProbe>, Failure> {
    let host = cfg.device_host.clone().ok_or_else(|| {
        fail_code(
            Status::BadRequest,
            "device_host_missing",
            ControlError::NotConfigured("PROVIEW_DEVICE_HOST").to_string(),
        )
    })?;
    let mut connection = proview_core::dvrip::client::Connection::connect(&host)
        .await
        .map_err(|e| fail(Status::BadGateway, format!("connect {host}: {e}")))?;
    let login = connection
        .login(&cfg.device_user, &cfg.device_pass)
        .await
        .map_err(|e| fail(Status::BadGateway, format!("login {host}: {e}")))?;
    let session = login.session;
    let device_type = login.device_type;
    let channels = login.channels;
    let alive_interval = login.alive_interval;
    let _ = connection.logout().await;

    let (video_codec, audio_codec, framerate) = match cfg.device_rtsp.as_deref() {
        Some(rtsp) => {
            let (rtsp_host, rtsp_port) = rtsp.rsplit_once(':').unwrap_or((rtsp, "554"));
            let port = rtsp_port.parse().unwrap_or(554);
            let info = proview_core::rtsp::describe(
                rtsp_host,
                port,
                "/",
                &cfg.device_user,
                &cfg.device_pass,
            )
            .await
            .map_err(|e| fail(Status::BadGateway, format!("rtsp {rtsp}: {e}")))?;
            (info.video_codec, info.audio_codec, info.framerate)
        }
        None => (None, None, None),
    };

    Ok(Json(DeviceProbe {
        ok: true,
        host,
        session,
        device_type,
        channels,
        alive_interval,
        video_codec,
        audio_codec,
        framerate,
    }))
}

#[get("/stream?<device>&<channel>&<quality>")]
async fn stream(
    device: String,
    channel: Option<u32>,
    quality: Option<String>,
    cfg: &State<Config>,
    store: &State<Store>,
    modes: &State<ModeStore>,
    cookies: &CookieJar<'_>,
) -> Result<Json<StreamTicket>, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    let router = Router::new(cfg, modes.mode_for(Some(&id)));
    let high = quality.as_deref() != Some("sub");
    router
        .stream(&vendor, &device, channel.unwrap_or(1), high)
        .await
        .map(Json)
        .map_err(|e| fail(status_for(&e), e.to_string()))
}

#[get("/cloud-path?<device>")]
async fn cloud_path(
    device: String,
    cfg: &State<Config>,
    store: &State<Store>,
    modes: &State<ModeStore>,
    cookies: &CookieJar<'_>,
) -> Result<Json<CloudPath>, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    let router = Router::new(cfg, modes.mode_for(Some(&id)));
    router
        .cloud_path(&vendor, &device)
        .await
        .map(Json)
        .map_err(|e| fail(status_for(&e), e.to_string()))
}

#[get("/ws/live?<device>&<channel>&<quality>&<codec>")]
#[allow(clippy::too_many_arguments)]
fn ws_live(
    ws: ws::WebSocket,
    device: String,
    channel: Option<u32>,
    quality: Option<String>,
    codec: Option<String>,
    cfg: &State<Config>,
    store: &State<Store>,
    live: &State<crate::live_state::LiveState>,
    cookies: &CookieJar<'_>,
) -> Result<ws::Channel<'static>, Failure> {
    let id = session_id(cookies)?;
    restore(store, &id)?;
    if device.contains('/') || device.contains("..") {
        return Err(fail(Status::BadRequest, "invalid device id"));
    }
    let channel = channel.unwrap_or(1);
    let sub_stream = quality.as_deref() == Some("sub");
    let target = rtsp_target_stream(cfg, sub_stream);
    let mode = match target {
        Some(_) => crate::stream::requested_mode(codec.as_deref()),
        None => LiveMode::Transcode,
    };
    let key = stream_key(&device, channel, sub_stream, mode, target.as_ref());
    let subscription = live.hub.subscribe(key.clone());
    let mut keep_alive = None;
    match target.clone() {
        Some(source) => {
            live.ensure_rtsp_source(key.clone(), source);
        }
        None => {
            let stem = if sub_stream { "sub" } else { "main" };
            let path = format!("{}/{device}_ch{channel}_{stem}.h264", cfg.fixture_dir);
            let plan = live
                .plan_for(&path, 25)
                .map_err(|e| fail(Status::NotFound, e))?;
            live.ensure_publisher(key.clone(), plan.clone());
            keep_alive = Some(plan);
        }
    }
    let session = crate::stream::LiveSession {
        key,
        subscription,
        restart: target,
        keep_alive,
    };
    Ok(crate::stream::channel(ws, live.inner().clone(), session))
}

#[get("/media/<device>/<channel>/<quality>")]
fn media_clip(
    device: String,
    channel: u32,
    quality: String,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<(rocket::http::ContentType, Vec<u8>), Failure> {
    let id = session_id(cookies)?;
    restore(store, &id)?;
    if device.contains('/') || device.contains("..") {
        return Err(fail(Status::BadRequest, "invalid device id"));
    }
    let stem = quality.trim_end_matches(".mp4");
    if stem != "main" && stem != "sub" {
        return Err(fail(Status::BadRequest, "quality must be main or sub"));
    }
    let path = format!("{}/{device}_ch{channel}_{stem}.h264", cfg.fixture_dir);
    let clip = std::fs::read(&path).map_err(|_| {
        fail(
            Status::NotFound,
            format!("no recorded clip for {device} ch{channel}"),
        )
    })?;
    let mp4 = proview_core::stream::annexb_to_fmp4(&clip, 25)
        .map_err(|e| fail(Status::UnprocessableEntity, e.to_string()))?;
    Ok((rocket::http::ContentType::new("video", "mp4"), mp4))
}

#[cfg(test)]
mod tests {
    use super::{rtsp_target_stream, stream_key, RTSP_CHANNEL};
    use proview_core::config::{Config, ProviderKind};
    use proview_core::live::LiveMode;

    fn config(rtsp: Option<&str>) -> Config {
        Config {
            provider: ProviderKind::Fixture,
            cloud_base: "https://pub-token.xmeye.net".to_string(),
            device_user: "admin".to_string(),
            device_pass: "device-password".to_string(),
            fixture_dir: "fixtures".to_string(),
            device_host: None,
            device_rtsp: rtsp.map(str::to_string),
            device_onvif: None,
            session_ttl_secs: 3600,
        }
    }

    #[test]
    fn every_requested_channel_names_the_one_rtsp_upstream() {
        let cfg = config(Some("192.0.2.10:554"));
        let target = rtsp_target_stream(&cfg, false).expect("rtsp target");
        let first = stream_key("cam-0001", 1, false, LiveMode::Passthrough, Some(&target));
        let second = stream_key("cam-0001", 4, false, LiveMode::Passthrough, Some(&target));
        assert_eq!(first, second);
        assert_eq!(first.device, "local");
        assert_eq!(first.channel, RTSP_CHANNEL);

        let fixture = stream_key("cam-0001", 4, true, LiveMode::Transcode, None);
        assert_eq!(fixture.device, "cam-0001");
        assert_eq!(fixture.channel, 4);
        assert!(fixture.sub_stream);
        assert_eq!(fixture.mode, LiveMode::Transcode);
    }

    #[test]
    fn the_rtsp_target_streams_the_channel_its_key_names() {
        let cfg = config(Some("192.0.2.10:554"));
        let main = rtsp_target_stream(&cfg, false).expect("rtsp target");
        let sub = rtsp_target_stream(&cfg, true).expect("rtsp target");
        assert!(main.path.contains(&format!("channel={RTSP_CHANNEL}&")));
        assert!(main.path.contains("stream=0.sdp"));
        assert!(sub.path.contains("stream=1.sdp"));
        assert_eq!(main.host, "192.0.2.10");
        assert_eq!(main.port, 554);
        assert_eq!(main.user, "admin");
        assert_eq!(main.password, "device-password");
        assert!(rtsp_target_stream(&config(None), false).is_none());
    }
}
