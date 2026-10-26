use std::time::Duration;

use rocket::http::{Cookie, CookieJar, SameSite, Status};
use rocket::response::content::{RawCss, RawHtml, RawJavaScript};
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::provider::{self, Device, ProviderError, StreamTicket};
use crate::session::{self, Store};

pub fn files() -> Vec<rocket::route::Route> {
    routes![index, app_js, style_css]
}

pub fn api() -> Vec<rocket::route::Route> {
    routes![login, logout, me, devices, stream, probe_device, health]
}

#[get("/")]
fn index() -> RawHtml<&'static str> {
    RawHtml(include_str!("../static/index.html"))
}

#[get("/app.js")]
fn app_js() -> RawJavaScript<&'static str> {
    RawJavaScript(include_str!("../static/app.js"))
}

#[get("/style.css")]
fn style_css() -> RawCss<&'static str> {
    RawCss(include_str!("../static/style.css"))
}

#[get("/health")]
fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "service": "proview-cctv-web" }))
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
pub struct DeviceProbe {
    pub ok: bool,
    pub host: String,
    pub session: u32,
    pub device_type: Option<String>,
}

#[post("/device/probe")]
async fn probe_device(cfg: &State<Config>) -> Result<Json<DeviceProbe>, Failure> {
    let host = cfg
        .device_host
        .clone()
        .ok_or_else(|| fail(Status::BadRequest, "PROVIEW_DEVICE_HOST is not configured"))?;
    let mut connection = crate::dvrip::client::Connection::connect(&host)
        .await
        .map_err(|e| fail(Status::BadGateway, format!("connect {host}: {e}")))?;
    let login = connection
        .login(&cfg.device_user, &cfg.device_pass)
        .await
        .map_err(|e| fail(Status::BadGateway, format!("login {host}: {e}")))?;
    let session = login.session;
    let device_type = login.device_type;
    let _ = connection.logout().await;
    Ok(Json(DeviceProbe {
        ok: true,
        host,
        session,
        device_type,
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
    let provider = provider::build(cfg);
    provider
        .devices(&vendor)
        .await
        .map(Json)
        .map_err(|e| fail(status_for(&e), e.to_string()))
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
    let provider = provider::build(cfg);
    let high = quality.as_deref() != Some("sub");
    provider
        .stream(&vendor, &device, channel.unwrap_or(1), high)
        .await
        .map(Json)
        .map_err(|e| fail(status_for(&e), e.to_string()))
}
