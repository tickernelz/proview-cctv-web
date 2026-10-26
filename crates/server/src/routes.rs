use std::time::Duration;

use futures_util::SinkExt;
use proview_core::config::Config;
use proview_core::live::StreamKey;
use proview_core::provider::{self, Device, ProviderError, StreamKind, StreamTicket};
use proview_core::session::{self, Store};
use rocket::http::{Cookie, CookieJar, SameSite, Status};
use rocket::response::content::{RawCss, RawHtml, RawJavaScript};
use rocket::serde::json::Json;
use rocket::State;
use rocket_ws as ws;
use serde::{Deserialize, Serialize};

pub fn files() -> Vec<rocket::route::Route> {
    routes![index, app_js, style_css, tokens_css, media_clip]
}

pub fn api() -> Vec<rocket::route::Route> {
    routes![
        login,
        logout,
        me,
        devices,
        stream,
        probe_device,
        rtsp_probe,
        health
    ]
}

pub fn live() -> Vec<rocket::route::Route> {
    routes![ws_live]
}

#[get("/")]
fn index() -> RawHtml<&'static str> {
    RawHtml(include_str!("../../../web/index.html"))
}

#[get("/app.js")]
fn app_js() -> RawJavaScript<&'static str> {
    RawJavaScript(include_str!("../../../web/app.js"))
}

#[get("/tokens.css")]
fn tokens_css() -> RawCss<&'static str> {
    RawCss(include_str!("../../../web/tokens.css"))
}

#[get("/style.css")]
fn style_css() -> RawCss<&'static str> {
    RawCss(include_str!("../../../web/style.css"))
}

#[get("/health")]
fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "service": "proview-server" }))
}

#[derive(Serialize)]
pub struct StatusBody {
    pub ok: bool,
    pub message: String,
}

type Failure = (Status, Json<StatusBody>);

fn fail(status: Status, message: impl Into<String>) -> Failure {
    (
        status,
        Json(StatusBody {
            ok: false,
            message: message.into(),
        }),
    )
}

fn status_for(err: &ProviderError) -> Status {
    match err {
        ProviderError::BadCredentials => Status::Unauthorized,
        ProviderError::UnknownDevice(_) => Status::NotFound,
        ProviderError::NotImplemented(_) => Status::NotImplemented,
        _ => Status::BadGateway,
    }
}

fn session_id(cookies: &CookieJar<'_>) -> Result<String, Failure> {
    cookies
        .get_private("pvw_session")
        .map(|c| c.value().to_string())
        .ok_or_else(|| fail(Status::Unauthorized, "not signed in"))
}

fn restore(store: &State<Store>, id: &str) -> Result<provider::VendorSession, Failure> {
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
    cookies: &CookieJar<'_>,
) -> Result<Json<StatusBody>, Failure> {
    let provider = provider::build(cfg);
    let vendor = provider
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
    Ok(Json(StatusBody {
        ok: true,
        message: "signed in".to_string(),
    }))
}

#[post("/logout")]
fn logout(store: &State<Store>, cookies: &CookieJar<'_>) -> Json<StatusBody> {
    if let Some(c) = cookies.get_private("pvw_session") {
        store.remove(c.value());
        cookies.remove_private(c);
    }
    Json(StatusBody {
        ok: true,
        message: "signed out".to_string(),
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

fn urlencoding_light(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                c.to_string()
            } else {
                let mut encoded = String::new();
                for byte in c.to_string().as_bytes() {
                    encoded.push_str(&format!("%{byte:02X}"));
                }
                encoded
            }
        })
        .collect()
}

fn rtsp_target(cfg: &Config) -> Option<proview_core::source::RtspTarget> {
    rtsp_target_stream(cfg, false)
}

fn rtsp_target_stream(cfg: &Config, sub_stream: bool) -> Option<proview_core::source::RtspTarget> {
    let target = cfg.device_rtsp.clone()?;
    let (host, port) = target.rsplit_once(':').unwrap_or((target.as_str(), "554"));
    let port = port.parse().unwrap_or(554);
    let path = if sub_stream {
        "/user=&password=&channel=1&stream=1.sdp?real_stream"
    } else {
        "/user=&password=&channel=1&stream=0.sdp?real_stream"
    };
    Some(
        proview_core::source::RtspTarget::from_host_port(host, port)
            .with_path(path)
            .with_credentials(&cfg.device_user, &cfg.device_pass),
    )
}

#[post("/device/rtsp")]
async fn rtsp_probe(cfg: &State<Config>) -> Result<Json<RtspReport>, Failure> {
    let target = cfg
        .device_rtsp
        .clone()
        .ok_or_else(|| fail(Status::BadRequest, "PROVIEW_DEVICE_RTSP is not configured"))?;
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
    cookies: &CookieJar<'_>,
) -> Result<Json<Vec<Device>>, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    if let Some(target) = rtsp_target(cfg) {
        return Ok(Json(vec![Device {
            id: "local".to_string(),
            name: format!("Camera at {}", target.host),
            model: "LAN camera".to_string(),
            online: true,
            channels: 1,
            lan_host: Some(format!("{}:{}", target.host, target.port)),
        }]));
    }
    let provider = provider::build(cfg);
    provider
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
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub framerate: Option<u32>,
}

#[post("/device/probe?<variant>")]
async fn probe_device(
    variant: Option<String>,
    cfg: &State<Config>,
) -> Result<Json<DeviceProbe>, Failure> {
    let host = cfg
        .device_host
        .clone()
        .ok_or_else(|| fail(Status::BadRequest, "PROVIEW_DEVICE_HOST is not configured"))?;
    let mut connection = proview_core::dvrip::client::Connection::connect(&host)
        .await
        .map_err(|e| fail(Status::BadGateway, format!("connect {host}: {e}")))?;
    let login = match variant.as_deref() {
        Some(name) => {
            connection
                .login_variant(&cfg.device_user, &cfg.device_pass, name)
                .await
        }
        None => connection.login(&cfg.device_user, &cfg.device_pass).await,
    }
    .map_err(|e| fail(Status::BadGateway, format!("login {host}: {e}")))?;
    let session = login.session;
    let device_type = login.device_type;
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
    cookies: &CookieJar<'_>,
) -> Result<Json<StreamTicket>, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    let _ = vendor;
    if rtsp_target(cfg).is_some() {
        let channel = channel.unwrap_or(1);
        let quality = quality.unwrap_or_else(|| "main".to_string());
        return Ok(Json(StreamTicket {
            kind: StreamKind::Ws,
            url: format!(
                "/ws/live?device={}&channel={channel}&quality={quality}",
                urlencoding_light(&device)
            ),
            expires_in: cfg.session_ttl_secs,
        }));
    }
    let provider = provider::build(cfg);
    let high = quality.as_deref() != Some("sub");
    provider
        .stream(&vendor, &device, channel.unwrap_or(1), high)
        .await
        .map(Json)
        .map_err(|e| fail(status_for(&e), e.to_string()))
}

#[get("/ws/live?<device>&<channel>&<quality>")]
#[allow(clippy::too_many_arguments)]
fn ws_live(
    ws: ws::WebSocket,
    device: String,
    channel: Option<u32>,
    quality: Option<String>,
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
    let key = StreamKey::new(device.clone(), channel, sub_stream);
    let subscription = live.hub.subscribe(key.clone());
    let mut keep_alive = None;
    match rtsp_target_stream(cfg, sub_stream) {
        Some(target) => {
            live.ensure_rtsp_source(key.clone(), target);
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
    let state = live.inner().clone();
    let proview_core::live::Subscription {
        init,
        last,
        mut receiver,
    } = subscription;

    let restart_target = rtsp_target_stream(cfg, sub_stream);
    Ok(ws.channel(move |mut stream| {
        Box::pin(async move {
            let _keep_alive = keep_alive;
            for _ in 0..240 {
                if init.lock().map(|slot| slot.is_some()).unwrap_or(false) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            if init.lock().map(|slot| slot.is_none()).unwrap_or(true) {
                if let Some(target) = restart_target.clone() {
                    let live = state.clone();
                    live.ensure_rtsp_source(key.clone(), target);
                    for _ in 0..240 {
                        if init.lock().map(|slot| slot.is_some()).unwrap_or(false) {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                }
            }
            let ready = init.lock().ok().and_then(|slot| slot.clone());
            match ready {
                Some(segment) => {
                    if stream
                        .send(ws::Message::Binary(segment.to_vec()))
                        .await
                        .is_err()
                    {
                        state.release(&key);
                        return Ok(());
                    }
                }
                None => {
                    let _ = stream
                        .send(ws::Message::Text(
                            "the live source did not produce video".to_string(),
                        ))
                        .await;
                    state.release(&key);
                    let _ = stream.send(ws::Message::Close(None)).await;
                    return Ok(());
                }
            }
            if let Some(segment) = last.lock().ok().and_then(|slot| slot.clone()) {
                if stream
                    .send(ws::Message::Binary(segment.to_vec()))
                    .await
                    .is_err()
                {
                    state.release(&key);
                    return Ok(());
                }
            }
            loop {
                match receiver.recv().await {
                    Ok(fragment) => {
                        if stream
                            .send(ws::Message::Binary(fragment.to_vec()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(
                            skipped,
                            "viewer fell behind, skipping to the next fragment"
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            state.release(&key);
            let _ = stream.send(ws::Message::Close(None)).await;
            Ok(())
        })
    }))
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
