use std::path::PathBuf;

use proview_core::config::Config;
use proview_core::lens::{frame_size_from_sps, FrameSize, FrameSizeSource, LensCapabilities};
use proview_core::media::nal::split_annexb;
use proview_core::model::{LensStreamInfo, LensView, LensViewRequest};
use proview_core::provider::VendorSession;
use proview_core::rtsp::RtspSession;
use proview_core::session::Store;
use proview_core::source::RtspTarget;
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;

const QUALITIES: [(&str, u8); 2] = [("main", 0), ("sub", 1)];
const CAPABILITIES_ENV: &str = "PROVIEW_LENS_CAPABILITIES";
const PTZ_ENV: &str = "PROVIEW_LENS_PTZ";

pub fn routes() -> Vec<rocket::route::Route> {
    routes![lens_view]
}

#[derive(Serialize)]
pub struct LensErrorBody {
    pub ok: bool,
    pub message: String,
}

type Failure = (Status, Json<LensErrorBody>);

fn fail(status: Status, message: impl Into<String>) -> Failure {
    (
        status,
        Json(LensErrorBody {
            ok: false,
            message: message.into(),
        }),
    )
}

fn session_id(cookies: &CookieJar<'_>) -> Result<String, Failure> {
    cookies
        .get_private("pvw_session")
        .map(|cookie| cookie.value().to_string())
        .ok_or_else(|| fail(Status::Unauthorized, "not signed in"))
}

fn restore(store: &State<Store>, id: &str) -> Result<VendorSession, Failure> {
    store
        .get(id)
        .ok_or_else(|| fail(Status::Unauthorized, "session expired"))
}

fn stream_info(
    quality: &str,
    codec: &str,
    size: FrameSize,
    source: FrameSizeSource,
) -> LensStreamInfo {
    LensStreamInfo {
        quality: quality.to_string(),
        codec: codec.to_string(),
        width: size.width,
        height: size.height,
        size_source: source,
    }
}

fn rtsp_target(config: &Config, channel: u32, stream: u8) -> Option<RtspTarget> {
    let target = config.device_rtsp.clone()?;
    let (host, port) = target.rsplit_once(':').unwrap_or((target.as_str(), "554"));
    let port = port.parse().unwrap_or(554);
    let path = format!("/user=&password=&channel={channel}&stream={stream}.sdp?real_stream");
    Some(
        RtspTarget::from_host_port(host, port)
            .with_path(&path)
            .with_credentials(&config.device_user, &config.device_pass),
    )
}

fn fixture_size(config: &Config, device: &str, channel: u32, quality: &str) -> Option<FrameSize> {
    if device.contains('/') || device.contains("..") {
        return None;
    }
    let path =
        PathBuf::from(&config.fixture_dir).join(format!("{device}_ch{channel}_{quality}.h264"));
    let clip = std::fs::read(path).ok()?;
    let sps = split_annexb(&clip)
        .into_iter()
        .find(|unit| unit.h264_type() == Some(7))?
        .data
        .to_vec();
    frame_size_from_sps("H264", &sps)
}

async fn probe_stream(
    config: &Config,
    device: &str,
    channel: u32,
    quality: &str,
    stream: u8,
    hint: Option<FrameSize>,
) -> LensStreamInfo {
    if let Some(target) = rtsp_target(config, channel, stream) {
        if let Ok(session) = RtspSession::open(
            &target.host,
            target.port,
            &target.path,
            &target.user,
            &target.password,
        )
        .await
        {
            if let Some(track) = session.video_track() {
                let measured = track
                    .sps
                    .as_deref()
                    .and_then(|sps| frame_size_from_sps(&track.codec, sps));
                if let Some(size) = measured {
                    return stream_info(quality, &track.codec, size, FrameSizeSource::SdpSps);
                }
                let codec = track.codec.clone();
                return match hint {
                    Some(size) => stream_info(quality, &codec, size, FrameSizeSource::ClientHint),
                    None => stream_info(
                        quality,
                        &codec,
                        FrameSize {
                            width: 0,
                            height: 0,
                        },
                        FrameSizeSource::Unknown,
                    ),
                };
            }
        }
    }
    if let Some(size) = fixture_size(config, device, channel, quality) {
        return stream_info(quality, "H264", size, FrameSizeSource::SdpSps);
    }
    match hint {
        Some(size) => stream_info(quality, "unknown", size, FrameSizeSource::ClientHint),
        None => stream_info(
            quality,
            "unknown",
            FrameSize {
                width: 0,
                height: 0,
            },
            FrameSizeSource::Unknown,
        ),
    }
}

fn declared_ptz_lens() -> Option<u32> {
    std::env::var(PTZ_ENV).ok()?.trim().parse().ok()
}

#[get("/lens?<device>&<channel>&<quality>&<width>&<height>")]
#[allow(clippy::too_many_arguments)]
async fn lens_view(
    device: String,
    channel: Option<u32>,
    quality: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    config: &State<Config>,
    store: &State<Store>,
    cookies: &CookieJar<'_>,
) -> Result<Json<LensView>, Failure> {
    let id = session_id(cookies)?;
    restore(store, &id)?;
    if device.contains('/') || device.contains("..") {
        return Err(fail(Status::BadRequest, "invalid device id"));
    }
    if let Some(name) = quality.as_deref() {
        if !QUALITIES.iter().any(|(known, _)| *known == name) {
            return Err(fail(Status::BadRequest, "quality must be main or sub"));
        }
    }
    let channel = channel.unwrap_or(1);
    let hint = match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => Some(FrameSize { width, height }),
        _ => None,
    };
    let mut streams = Vec::new();
    for (name, index) in QUALITIES {
        if let Some(wanted) = quality.as_deref() {
            if wanted != name {
                continue;
            }
        }
        streams.push(probe_stream(config, &device, channel, name, index, hint).await);
    }
    let capabilities =
        LensCapabilities::from_flag_list(&std::env::var(CAPABILITIES_ENV).unwrap_or_default());
    Ok(Json(LensView::build(LensViewRequest {
        device_id: &device,
        channel,
        streams,
        capabilities,
        ptz: declared_ptz_lens(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proview_core::config::ProviderKind;
    use rocket::local::blocking::Client;

    static FIXTURE_SLOT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    const DUAL_LENS_CLIP: &[u8] =
        include_bytes!("../../../fixtures/dual-lens/dual-lens-stacked-640x720.h264");

    fn fixture_config() -> (PathBuf, Config) {
        let slot = FIXTURE_SLOT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("zha3907-lens-{}-{slot}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("fixture dir");
        std::fs::write(
            dir.join("account.json"),
            r#"{"email":"fixture@example.invalid","password":"fixture-password"}"#,
        )
        .expect("account fixture");
        std::fs::write(
            dir.join("devices.json"),
            r#"[{"id":"fixture-dev-0001","name":"Fixture camera","model":"XMEye-Fixture","online":true,"channels":1,"lan_host":"127.0.0.1:34567"}]"#,
        )
        .expect("device fixture");
        std::fs::write(dir.join("fixture-dev-0001_ch1_main.h264"), DUAL_LENS_CLIP)
            .expect("main clip");
        std::fs::write(dir.join("fixture-dev-0001_ch1_sub.h264"), DUAL_LENS_CLIP)
            .expect("sub clip");
        let config = Config {
            provider: ProviderKind::Fixture,
            cloud_base: String::new(),
            device_user: "admin".to_string(),
            device_pass: String::new(),
            fixture_dir: dir.to_string_lossy().to_string(),
            device_onvif: None,
            device_host: None,
            device_rtsp: None,
            session_ttl_secs: 60,
        };
        (dir, config)
    }

    fn signed_in_client(config: Config) -> Client {
        let figment = rocket::Config::figment()
            .merge(("secret_key", "aGVsbG8gd29ybGQgdGhpcyBpcyBhIHRlc3Qga2V5ISE="));
        let client = Client::tracked(crate::app::build_with(figment, config)).expect("rocket");
        let status = client
            .post("/api/login")
            .json(&serde_json::json!({
                "email": "fixture@example.invalid",
                "password": "fixture-password"
            }))
            .dispatch()
            .status();
        assert_eq!(status, Status::Ok);
        client
    }

    #[test]
    fn the_contract_is_served_with_two_stacked_lenses() {
        let (dir, config) = fixture_config();
        let client = signed_in_client(config);
        let response = client
            .get("/api/lens?device=fixture-dev-0001&channel=1")
            .dispatch();
        assert_eq!(response.status(), Status::Ok);
        let raw = response.into_string().expect("body");
        let view: LensView = serde_json::from_str(&raw).expect("contract parses");
        assert_eq!(view.device_id, "fixture-dev-0001");
        assert_eq!(view.channel, 1);
        assert_eq!(view.streams.len(), 2);
        assert_eq!(view.streams[0].quality, "main");
        assert_eq!(view.streams[0].width, 640);
        assert_eq!(view.streams[0].height, 720);
        assert_eq!(view.lens_count, 2);
        assert_eq!(
            view.count_source,
            proview_core::lens::LensCountSource::Geometry
        );
        assert!(view.crops_verified);
        assert_eq!(view.lenses.len(), 2);
        assert_eq!(view.lenses[0].crop.h, 0.5);
        assert_eq!(view.lenses[1].crop.y, 0.5);
        let value: serde_json::Value = serde_json::from_str(&raw).expect("json");
        assert_eq!(value["lenses"][0]["label"], "top");
        assert_eq!(value["lenses"][1]["label"], "bottom");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_lens_route_requires_a_session() {
        let (dir, config) = fixture_config();
        let figment = rocket::Config::figment()
            .merge(("secret_key", "aGVsbG8gd29ybGQgdGhpcyBpcyBhIHRlc3Qga2V5ISE="));
        let client = Client::tracked(crate::app::build_with(figment, config)).expect("rocket");
        let response = client.get("/api/lens?device=fixture-dev-0001").dispatch();
        assert_eq!(response.status(), Status::Unauthorized);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unknown_quality_is_rejected() {
        let (dir, config) = fixture_config();
        let client = signed_in_client(config);
        let response = client
            .get("/api/lens?device=fixture-dev-0001&quality=medium")
            .dispatch();
        assert_eq!(response.status(), Status::BadRequest);
        let _ = std::fs::remove_dir_all(dir);
    }
}
