use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use proview_core::config::Config;
use proview_core::dvrip::talk::{TalkLink, TalkSession};
use proview_core::media::audio;
use proview_core::provider::cloud::CloudProvider;
use proview_core::provider::{ProviderError, VendorSession};
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

async fn relay_link(
    cloud: &CloudProvider,
    vendor: &VendorSession,
    device: &str,
) -> Result<TalkLink, TalkFailure> {
    match cloud.open_talk(vendor, device).await {
        Ok(connection) => Ok(TalkLink::from_connection(connection)),
        Err(error) => Err(relay_failure(device, error)),
    }
}

fn relay_failure(device: &str, error: ProviderError) -> TalkFailure {
    match error {
        ProviderError::UnknownDevice(_) => TalkFailure::path(format!(
            "{device} is not on this account; pick a camera this sign-in owns"
        )),
        ProviderError::Misconfigured(message) => TalkFailure::path(message),
        ProviderError::Upstream(message) if message.contains("no rps id") => TalkFailure::path(
            format!("{device} carries no relay id in the device list, so the vendor relay cannot be claimed"),
        ),
        ProviderError::Upstream(message) => {
            TalkFailure::device(format!("the vendor relay did not answer: {message}"))
        }
        other => TalkFailure::device(format!("the vendor relay did not answer: {other}")),
    }
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

    fn ready(&self) -> bool {
        !self.user.trim().is_empty() && !self.pass.trim().is_empty()
    }

    fn refusal(&self, device: &str) -> Option<TalkFailure> {
        proview_core::device::serial_refusal(&self.serial, device).map(TalkFailure::path)
    }
}

async fn direct_link(address: &str, account: &TalkAccount) -> Result<TalkLink, String> {
    let mut link = match tokio::time::timeout(DIRECT_WAIT, TalkLink::connect(address)).await {
        Ok(Ok(link)) => link,
        Ok(Err(error)) => return Err(format!("{address}: {error}")),
        Err(_) => return Err(format!("{address}: no answer in {DIRECT_WAIT:?}")),
    };
    match link.login(&account.user, &account.pass).await {
        Ok(()) => Ok(link),
        Err(error) => Err(format!("{address}: {error}")),
    }
}

async fn open_link(
    cfg: &Config,
    cloud: &CloudProvider,
    vendor: &VendorSession,
    device: Option<&str>,
    account: &TalkAccount,
) -> Result<(TalkLink, &'static str), TalkFailure> {
    let named = device.map(str::trim).filter(|name| !name.is_empty());
    let direct = cfg
        .device_host
        .clone()
        .filter(|host| !host.trim().is_empty());
    let token = vendor.token.as_deref().map(str::trim).unwrap_or("");
    let mut direct_attempted = false;
    let mut direct_error = None;
    if let Some(address) = direct.as_deref() {
        if account.ready() {
            if let Some(refusal) = named.and_then(|name| account.refusal(name)) {
                return Err(refusal);
            }
            direct_attempted = true;
            match direct_link(address, account).await {
                Ok(link) => return Ok((link, "lan")),
                Err(error) => direct_error = Some(error),
            }
        } else {
            direct_error = Some(format!(
                "{address}: no camera user and password; set them under Settings, Configuration, Local network"
            ));
        }
    }
    let relay_failure = match named {
        Some(name) if !token.is_empty() => match relay_link(cloud, vendor, name).await {
            Ok(link) => return Ok((link, "relay")),
            Err(failure) => failure,
        },
        Some(_) => TalkFailure::path(
            "this sign-in has no cloud session; sign in through the cloud, or set a direct camera address"
                .to_string(),
        ),
        None => TalkFailure::path("no camera was named in the request".to_string()),
    };
    let direct_reason = match direct_error {
        Some(error) => format!("direct: {error}"),
        None => "direct: no camera address is set".to_string(),
    };
    let message = format!(
        "no path to the camera ({direct_reason}; vendor relay: {})",
        relay_failure.message
    );
    if relay_failure.scope == "path" && !direct_attempted {
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
    cloud: &State<Arc<CloudProvider>>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<ws::Channel<'static>, Failure> {
    let id = session_id(cookies)?;
    let vendor = restore(store, &id)?;
    let config = cfg.inner().clone();
    let provider = cloud.inner().clone();

    Ok(ws.channel(move |mut stream| {
        Box::pin(async move {
            let account = TalkAccount::from_config(&config);
            let (link, path) =
                match open_link(&config, &provider, &vendor, device.as_deref(), &account).await {
                    Ok(pair) => pair,
                    Err(failure) => {
                        let _ = stream
                            .send(ws::Message::Text(json_error(
                                failure.scope,
                                &failure.message,
                            )))
                            .await;
                        let _ = stream.send(ws::Message::Close(None)).await;
                        return Ok(());
                    }
                };
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
        let vendor = VendorSession {
            account: "tester".to_string(),
            token: token.map(str::to_string),
        };
        let cloud = CloudProvider::new(cfg);
        match open_link(cfg, &cloud, &vendor, device, account).await {
            Ok(_) => panic!("a path was found"),
            Err(failure) => failure,
        }
    }

    fn message(failure: TalkFailure) -> String {
        failure.message
    }

    #[test]
    fn relay_failures_scope_a_configuration_mistake_as_path() {
        let failure = relay_failure("ABC123", ProviderError::UnknownDevice("ABC123".into()));
        assert_eq!(failure.scope, "path");
        assert!(failure.message.contains("not on this account"));
        let failure = relay_failure("ABC123", ProviderError::Misconfigured("no identity".into()));
        assert_eq!(failure.scope, "path");
        assert_eq!(failure.message, "no identity");
        let failure = relay_failure(
            "ABC123",
            ProviderError::Upstream(
                "the device list carries no rps id, so the relay cannot be asked".into(),
            ),
        );
        assert_eq!(failure.scope, "path");
        assert!(failure.message.contains("no relay id"));
    }

    #[test]
    fn relay_failures_scope_a_live_tunnel_as_device() {
        let failure = relay_failure(
            "ABC123",
            ProviderError::Upstream("relay handshake failed; retry: relay refused".into()),
        );
        assert_eq!(failure.scope, "device");
        assert!(failure.message.contains("vendor relay"));
        assert!(failure.message.contains("relay refused"));
        let failure = relay_failure("ABC123", ProviderError::LoginRefused("ret 117".into()));
        assert_eq!(failure.scope, "device");
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
        cfg.device_host = Some("127.0.0.1:1".to_string());
        let failure = failure_of(&cfg, Some("token"), Some("serial-two")).await;
        assert_eq!(failure.scope, "path");
        let text = message(failure);
        assert!(
            text.contains("camera credentials belong to serial-one, not to serial-two"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn a_direct_address_needs_the_configured_serial() {
        let mut cfg = Config::from_env();
        cfg.device_host = Some("127.0.0.1:1".to_string());
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
    async fn a_direct_address_without_credentials_names_the_missing_password() {
        let mut cfg = Config::from_env();
        cfg.device_host = Some("127.0.0.1:1".to_string());
        let mut blank = account();
        blank.pass = String::new();
        let failure = failure_with(&cfg, None, Some("serial-one"), &blank).await;
        assert_eq!(failure.scope, "path");
        let text = message(failure);
        assert!(text.contains("no camera user and password"), "{text}");
        assert!(text.contains("no cloud session"), "{text}");
    }

    #[tokio::test]
    async fn the_cloud_path_never_reads_the_camera_password() {
        let mut cfg = Config::from_env();
        cfg.device_host = None;
        cfg.cloud_base = "http://127.0.0.1:1".to_string();
        let mut blank = account();
        blank.user = String::new();
        blank.pass = String::new();
        let failure = failure_with(&cfg, Some("token"), Some("serial-one"), &blank).await;
        let text = message(failure);
        assert!(!text.contains("camera user and password"), "{text}");
        assert!(text.contains("vendor relay"), "{text}");
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
    }
}
