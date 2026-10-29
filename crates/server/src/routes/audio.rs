use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use proview_core::config::Config;
use proview_core::dvrip::talk::{TalkLink, TalkSession};
use proview_core::media::audio;
use proview_core::session::Store;
use rocket::http::CookieJar;
use rocket::State;
use rocket_ws as ws;
use serde::{Deserialize, Serialize};

use super::{restore, session_id, Failure};

pub fn routes() -> Vec<rocket::route::Route> {
    routes![ws_talk]
}

#[derive(Debug, Serialize)]
pub struct TalkReady {
    pub kind: &'static str,
    pub codec: &'static str,
    pub channel: Option<u8>,
    pub rate: u32,
    pub frame: usize,
    pub claim_ms: u64,
    pub path: &'static str,
}

#[derive(Debug, Serialize)]
pub struct TalkReport {
    pub kind: &'static str,
    pub frames: u64,
    pub bytes: u64,
    pub chunks: u64,
    pub dropped: u64,
    pub seconds: f64,
    pub frames_per_second: f64,
    pub claim_ms: u64,
    pub write_ms_mean: f64,
    pub write_ms_max: f64,
    pub inbound_bytes: u64,
    pub inbound_frames: u64,
}

#[derive(Debug, Deserialize)]
struct Control {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Default)]
struct Meter {
    chunks: u64,
    frames: u64,
    write_total: Duration,
    write_max: Duration,
}

const DIRECT_WAIT: Duration = Duration::from_secs(3);

struct TalkFailure {
    scope: &'static str,
    message: String,
}

impl TalkFailure {
    fn path(message: impl Into<String>) -> Self {
        Self {
            scope: "path",
            message: message.into(),
        }
    }

    fn device(message: impl Into<String>) -> Self {
        Self {
            scope: "device",
            message: message.into(),
        }
    }
}

async fn relay_link(cfg: &Config, token: &str, device: &str) -> Result<TalkLink, TalkFailure> {
    use proview_core::cloud::envelope::{EnvelopeClient, Identity, MISSING_CLOUD_IDENTITY};
    use proview_core::cloud::status::{auth_code_of, StatusClient, STATUS_HOST};
    use proview_core::provider::{p2p, rps};

    let identity = Identity::resolve();
    if !identity.is_complete() {
        return Err(TalkFailure::path(MISSING_CLOUD_IDENTITY));
    }
    let client = EnvelopeClient::with_base(identity, cfg.cloud_base.clone());
    let devices = client
        .device_list(token)
        .await
        .map_err(|error| TalkFailure::device(format!("device list failed: {error}")))?;
    let found = devices
        .iter()
        .find(|entry| entry.uuid == device)
        .ok_or_else(|| {
            TalkFailure::path(format!(
                "{device} is not on this account; pick a camera this sign-in owns"
            ))
        })?;
    let serial = found.uuid.clone();
    let rps_id = found
        .extra
        .get("rps")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    if rps_id.is_empty() {
        return Err(TalkFailure::path(format!(
            "{serial} carries no relay id in the device list, so the vendor relay cannot be claimed"
        )));
    }
    let status = StatusClient::new(STATUS_HOST)
        .query(&serial, &auth_code_of(found))
        .await
        .map_err(|error| TalkFailure::device(format!("device status failed: {error}")))?;
    if !status.online {
        return Err(TalkFailure::device(format!(
            "{serial} is offline in the vendor cloud"
        )));
    }
    let (host, port) = status
        .access_server()
        .ok_or_else(|| TalkFailure::device("no status cluster reported an access server"))?;
    let session = rps::session_id();
    let client_token = rps::client_token();
    let body = p2p::agent_request(
        &rps_id,
        &client_token,
        &serial,
        p2p::AGENT_SERVICE_MEDIA,
        &session,
        p2p::AGENT_STEP_MEDIA,
    );
    let agent = rps::AccessClient::new()
        .assign(&host, port, &body)
        .await
        .map_err(|error| {
            TalkFailure::device(format!("the access server refused the relay: {error}"))
        })?;
    let frame = p2p::handshake_frame(&rps_id, &client_token, &serial, &session);
    let tunnel = rps::open(&agent, &frame).await.map_err(|error| {
        TalkFailure::device(format!("the vendor relay did not accept: {error}"))
    })?;
    if !tunnel.reply.accepted() {
        return Err(TalkFailure::device(format!(
            "the vendor relay answered {}",
            tunnel.reply.error_num
        )));
    }
    Ok(TalkLink::from_stream(tunnel.stream))
}

struct TalkAccount {
    serial: String,
    user: String,
    pass: String,
}

impl TalkAccount {
    fn from_config(cfg: &Config) -> Self {
        Self {
            serial: proview_core::device::configured_serial(),
            user: cfg.device_user.clone(),
            pass: cfg.device_pass.clone(),
        }
    }

    fn refusal(&self, device: &str) -> Option<TalkFailure> {
        if self.user.trim().is_empty() || self.pass.trim().is_empty() {
            return Some(TalkFailure::path(format!(
                "no camera user and password for {}: set them under Settings, Configuration, Local network",
                device.trim()
            )));
        }
        proview_core::device::serial_refusal(&self.serial, device).map(TalkFailure::path)
    }
}

async fn open_link(
    cfg: &Config,
    token: Option<&str>,
    device: Option<&str>,
    account: &TalkAccount,
) -> Result<(TalkLink, &'static str), TalkFailure> {
    if let Some(name) = device {
        if let Some(refusal) = account.refusal(name) {
            return Err(refusal);
        }
    }
    let direct = cfg
        .device_host
        .clone()
        .filter(|host| !host.trim().is_empty());
    let mut direct_error = None;
    if let Some(address) = direct {
        match tokio::time::timeout(DIRECT_WAIT, TalkLink::connect(&address)).await {
            Ok(Ok(link)) => return Ok((link, "lan")),
            Ok(Err(error)) => direct_error = Some(format!("{address}: {error}")),
            Err(_) => direct_error = Some(format!("{address}: no answer in {DIRECT_WAIT:?}")),
        }
    }
    let (relay_reason, relay_scope) = match (device, token) {
        (None, _) => (
            "no camera was named in the request".to_string(),
            "path",
        ),
        (Some(device), Some(token)) => match relay_link(cfg, token, device).await {
            Ok(link) => return Ok((link, "relay")),
            Err(failure) => (failure.message, failure.scope),
        },
        (Some(_), None) => (
            "this sign-in has no cloud session; sign in through the cloud, or set a direct camera address".to_string(),
            "path",
        ),
    };
    let tried_direct = direct_error.is_some();
    let direct_reason = match direct_error {
        Some(error) => format!("direct: {error}"),
        None => "direct: no camera address is set".to_string(),
    };
    let message = format!("no path to the camera ({direct_reason}; vendor relay: {relay_reason})");
    if relay_scope == "path" && !tried_direct {
        Err(TalkFailure::path(message))
    } else {
        Err(TalkFailure::device(message))
    }
}

#[get("/ws/talk?<channel>&<device>")]
async fn ws_talk(
    ws: ws::WebSocket,
    channel: Option<u8>,
    device: Option<String>,
    cfg: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<ws::Channel<'static>, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    let token = vendor.token.clone();
    let config = cfg.inner().clone();
    let user = config.device_user.clone();
    let password = config.device_pass.clone();

    Ok(ws.channel(move |mut stream| {
        Box::pin(async move {
            if password.trim().is_empty() {
                let _ = stream
                    .send(ws::Message::Text(json_error(
                        "path",
                        "the camera password is not set, so the talk login would fail and the device locks the source IP after repeated attempts; set it under Settings, Configuration, Local network",
                    )))
                    .await;
                let _ = stream.send(ws::Message::Close(None)).await;
                return Ok(());
            }
            let account = TalkAccount::from_config(&config);
            let (mut link, path) =
                match open_link(&config, token.as_deref(), device.as_deref(), &account).await {
                Ok(pair) => pair,
                Err(failure) => {
                    let _ = stream
                        .send(ws::Message::Text(json_error(failure.scope, &failure.message)))
                        .await;
                    let _ = stream.send(ws::Message::Close(None)).await;
                    return Ok(());
                }
            };
            if let Err(error) = link.login(&user, &password).await {
                let _ = stream
                    .send(ws::Message::Text(json_error(
                        "device",
                        &format!("the camera refused the talk login: {error}"),
                    )))
                    .await;
                link.close().await;
                let _ = stream.send(ws::Message::Close(None)).await;
                return Ok(());
            }
            let mut talk = match TalkSession::open(link, channel).await {
                Ok(talk) => talk,
                Err(error) => {
                    let _ = stream
                        .send(ws::Message::Text(json_error(
                            "device",
                            &format!("the camera refused the talk channel: {error}"),
                        )))
                        .await;
                    let _ = stream.send(ws::Message::Close(None)).await;
                    return Ok(());
                }
            };
            let ready = TalkReady {
                kind: "ready",
                codec: "g711a",
                channel,
                rate: audio::SAMPLE_RATE,
                frame: audio::FRAME_SAMPLES,
                claim_ms: talk.stats().claim_ms,
                path,
            };
            if stream
                .send(ws::Message::Text(
                    serde_json::to_string(&ready).unwrap_or_default(),
                ))
                .await
                .is_err()
            {
                let _ = talk.stop().await;
                let _ = talk.logout().await;
                return Ok(());
            }

            let mut meter = Meter::default();
            let mut last_report = Instant::now();
            while let Some(message) = stream.next().await {
                match message {
                    Ok(ws::Message::Binary(payload)) => {
                        let samples = audio::from_pcm16le(&payload);
                        let alaw = audio::encode(&samples);
                        let started = Instant::now();
                        match talk.send_alaw(&alaw).await {
                            Ok(0) => {}
                            Ok(frames) => {
                                let spent = started.elapsed();
                                meter.frames += frames as u64;
                                meter.chunks += 1;
                                meter.write_total += spent;
                                meter.write_max = meter.write_max.max(spent);
                            }
                            Err(error) => {
                                let _ = stream
                                    .send(ws::Message::Text(json_error(
                                        "device",
                                        &format!("the camera stopped accepting audio: {error}"),
                                    )))
                                    .await;
                                break;
                            }
                        }
                        if last_report.elapsed() >= Duration::from_millis(1000) {
                            last_report = Instant::now();
                            let report = report(&talk, &meter);
                            if stream
                                .send(ws::Message::Text(
                                    serde_json::to_string(&report).unwrap_or_default(),
                                ))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                    Ok(ws::Message::Text(text)) => {
                        if let Ok(control) = serde_json::from_str::<Control>(&text) {
                            if control.kind == "stop" {
                                break;
                            }
                        }
                    }
                    Ok(ws::Message::Close(_)) => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }

            let final_report = report(&talk, &meter);
            let _ = stream
                .send(ws::Message::Text(
                    serde_json::to_string(&final_report).unwrap_or_default(),
                ))
                .await;
            let _ = talk.stop().await;
            let _ = talk.logout().await;
            let _ = stream.send(ws::Message::Close(None)).await;
            Ok(())
        })
    }))
}

fn report(talk: &TalkSession, meter: &Meter) -> TalkReport {
    let stats = talk.stats();
    let mean = if meter.chunks == 0 {
        0.0
    } else {
        meter.write_total.as_secs_f64() * 1000.0 / meter.chunks as f64
    };
    TalkReport {
        kind: "report",
        frames: stats.frames,
        bytes: stats.bytes,
        chunks: meter.chunks,
        dropped: stats.dropped,
        seconds: stats.seconds,
        frames_per_second: if stats.seconds > 0.0 {
            stats.frames as f64 / stats.seconds
        } else {
            0.0
        },
        claim_ms: stats.claim_ms,
        write_ms_mean: mean,
        write_ms_max: meter.write_max.as_secs_f64() * 1000.0,
        inbound_bytes: stats.inbound_bytes,
        inbound_frames: stats.inbound_frames,
    }
}

fn json_error(scope: &str, message: &str) -> String {
    serde_json::json!({ "kind": "error", "scope": scope, "message": message }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> TalkAccount {
        TalkAccount {
            serial: "serial-one".to_string(),
            user: "admin".to_string(),
            pass: "pass".to_string(),
        }
    }

    async fn failure_of(cfg: &Config, token: Option<&str>, device: Option<&str>) -> TalkFailure {
        failure_with(cfg, token, device, &account()).await
    }

    async fn failure_with(
        cfg: &Config,
        token: Option<&str>,
        device: Option<&str>,
        account: &TalkAccount,
    ) -> TalkFailure {
        match open_link(cfg, token, device, account).await {
            Ok(_) => panic!("a path was found"),
            Err(failure) => failure,
        }
    }

    fn message(failure: TalkFailure) -> String {
        failure.message
    }

    #[tokio::test]
    async fn no_configured_path_names_both_reasons() {
        let mut cfg = Config::from_env();
        cfg.device_host = None;
        let failure = failure_of(&cfg, None, None).await;
        assert_eq!(failure.scope, "path");
        let text = message(failure);
        assert!(text.contains("no camera address is set"), "{text}");
        assert!(text.contains("no camera was named"), "{text}");
        assert!(!text.contains("PROVIEW_"), "{text}");
    }

    #[tokio::test]
    async fn a_bound_device_without_a_cloud_session_says_so() {
        let mut cfg = Config::from_env();
        cfg.device_host = None;
        let failure = failure_of(&cfg, None, Some("serial-one")).await;
        assert_eq!(failure.scope, "path");
        let text = message(failure);
        assert!(text.contains("no cloud session"), "{text}");
    }

    #[tokio::test]
    async fn a_dead_lan_address_falls_through_and_reports_both_attempts() {
        let mut cfg = Config::from_env();
        cfg.device_host = Some("127.0.0.1:1".to_string());
        let failure = failure_of(&cfg, None, Some("serial-one")).await;
        assert_eq!(
            failure.scope, "device",
            "a camera that is briefly unreachable must not disable the button"
        );
        let text = message(failure);
        assert!(text.contains("direct: 127.0.0.1:1"), "{text}");
        assert!(text.contains("vendor relay:"), "{text}");
    }

    #[tokio::test]
    async fn the_credentials_never_travel_to_another_camera() {
        let mut cfg = Config::from_env();
        cfg.device_host = None;
        let failure = failure_of(&cfg, Some("token"), Some("serial-two")).await;
        assert_eq!(failure.scope, "path");
        let text = message(failure);
        assert!(
            text.contains("camera credentials belong to serial-one, not to serial-two"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn a_named_camera_needs_the_configured_serial() {
        let mut cfg = Config::from_env();
        cfg.device_host = None;
        let mut unbound = account();
        unbound.serial = String::new();
        let failure = failure_with(&cfg, Some("token"), Some("serial-one"), &unbound).await;
        assert_eq!(failure.scope, "path");
        let text = message(failure);
        assert!(text.contains("no camera serial is configured"), "{text}");
        assert!(text.contains("serial-one"), "{text}");
        assert!(!text.contains("PROVIEW_"), "{text}");
    }

    #[tokio::test]
    async fn a_named_camera_needs_a_user_and_password() {
        let mut cfg = Config::from_env();
        cfg.device_host = None;
        let mut blank = account();
        blank.pass = String::new();
        let failure = failure_with(&cfg, Some("token"), Some("serial-one"), &blank).await;
        assert_eq!(failure.scope, "path");
        let text = message(failure);
        assert!(text.contains("no camera user and password"), "{text}");
    }

    #[tokio::test]
    async fn the_local_camera_skips_the_serial_guard() {
        let mut cfg = Config::from_env();
        cfg.device_host = Some("127.0.0.1:1".to_string());
        let failure = failure_of(&cfg, None, Some(proview_core::device::LOCAL_DEVICE)).await;
        assert_eq!(failure.scope, "device");
        let text = message(failure);
        assert!(text.contains("direct: 127.0.0.1:1"), "{text}");
        assert!(!text.contains("camera credentials belong"), "{text}");
    }

    #[tokio::test]
    async fn a_static_refusal_still_scopes_as_path_without_a_direct_attempt() {
        let mut cfg = Config::from_env();
        cfg.device_host = None;
        let failure = failure_of(&cfg, Some("token"), None).await;
        assert_eq!(failure.scope, "path");
        let text = message(failure);
        assert!(text.contains("no camera was named"), "{text}");

        let mut cfg = Config::from_env();
        cfg.device_host = None;
        let failure = failure_of(&cfg, Some("token"), Some("serial-two")).await;
        assert_eq!(failure.scope, "path");
    }
}
