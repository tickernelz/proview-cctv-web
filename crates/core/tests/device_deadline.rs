use std::time::{Duration, Instant};

use proview_core::device::{ControlError, DeviceControl, DvripControl, Lens};
use proview_core::dvrip::frame::Header;
use proview_core::dvrip::{msg, HEADER_LEN};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const WINDOW_MS: &str = "300";
const WINDOW: Duration = Duration::from_millis(300);
const OUTER: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Silent {
    Ability,
    ConfigGet,
    PtzControl,
    Snap,
    Logout,
}

fn control(device: &str) -> DvripControl {
    std::env::set_var("PROVIEW_DVRIP_WINDOW_MS", WINDOW_MS);
    DvripControl::new(&proview_core::config::Config {
        provider: proview_core::config::ProviderKind::Fixture,
        cloud_base: "https://pub-token.xmeye.net".to_string(),
        device_user: "admin".to_string(),
        device_pass: "device-password".to_string(),
        fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
        device_host: Some(device.to_string()),
        device_onvif: None,
        device_rtsp: None,
        session_ttl_secs: 3600,
    })
    .expect("the device host is set")
}

async fn fake(silent: Silent) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(serve(socket, silent));
        }
    });
    address
}

fn frame(header: &Header, payload: &[u8]) -> Vec<u8> {
    let mut out = Header {
        session: 0x1122_3344,
        sequence: header.sequence,
        channel: 0,
        end: true,
        id: header.id.wrapping_add(1),
        length: payload.len() as u32,
    }
    .encode()
    .to_vec();
    out.extend_from_slice(payload);
    out
}

fn reply(header: &Header, value: &Value) -> Vec<u8> {
    let mut payload = value.to_string().into_bytes();
    payload.push(b'\n');
    payload.push(0);
    frame(header, &payload)
}

fn requested_name(body: &[u8]) -> Option<String> {
    let mut end = body.len();
    while end > 0 && matches!(body[end - 1], 0 | b'\n' | b'\r') {
        end -= 1;
    }
    let value = serde_json::from_slice::<Value>(&body[..end]).ok()?;
    value
        .get("Name")
        .and_then(Value::as_str)
        .map(str::to_string)
}

async fn serve(mut socket: TcpStream, silent: Silent) {
    loop {
        let mut header_bytes = [0u8; HEADER_LEN];
        if socket.read_exact(&mut header_bytes).await.is_err() {
            return;
        }
        let header = match Header::decode(&header_bytes) {
            Ok(header) => header,
            Err(_) => return,
        };
        let mut body = vec![0u8; header.length as usize];
        if header.length > 0 && socket.read_exact(&mut body).await.is_err() {
            return;
        }
        let goes_silent = match silent {
            Silent::Ability => header.id == msg::ABILITY,
            Silent::ConfigGet => header.id == msg::CONFIG_GET,
            Silent::PtzControl => header.id == msg::PTZ_CONTROL,
            Silent::Snap => header.id == msg::SNAP,
            Silent::Logout => header.id == msg::LOGOUT,
        };
        if goes_silent {
            std::future::pending::<()>().await;
            return;
        }
        let out = match header.id {
            msg::LOGIN => reply(&header, &json!({ "Ret": 100, "SessionID": "0x11223344" })),
            msg::SNAP => frame(&header, &[0xff, 0xd8, 0xff, 0x00, 0x01]),
            msg::CONFIG_GET => match requested_name(&body) {
                Some(name) => reply(&header, &json!({ "Ret": 100, name: [{}] })),
                None => reply(&header, &json!({ "Ret": 100 })),
            },
            _ => reply(&header, &json!({ "Ret": 100 })),
        };
        if socket.write_all(&out).await.is_err() {
            return;
        }
    }
}

async fn timed<T, F>(label: &str, call: F) -> (Result<T, ControlError>, Duration)
where
    F: std::future::Future<Output = Result<T, ControlError>>,
{
    let started = Instant::now();
    let outcome = tokio::time::timeout(OUTER, call)
        .await
        .unwrap_or_else(|_| panic!("{label} never returned inside {OUTER:?}"));
    (outcome, started.elapsed())
}

fn assert_window(
    label: &str,
    outcome: Result<impl std::fmt::Debug, ControlError>,
    elapsed: Duration,
) {
    let error = match outcome {
        Ok(_) => panic!("{label} answered instead of timing out"),
        Err(error) => error,
    };
    let text = error.to_string();
    assert!(text.contains("did not answer inside"), "{label}: {text}");
    assert!(
        elapsed >= WINDOW && elapsed < OUTER,
        "{label} returned after {elapsed:?}, expected inside {WINDOW:?}..{OUTER:?}"
    );
}

#[tokio::test]
async fn a_silent_capability_read_times_out() {
    let control = control(&fake(Silent::Ability).await);
    let (outcome, elapsed) = timed("capabilities", control.capabilities()).await;
    assert_window("capabilities", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_tour_listing_times_out() {
    let control = control(&fake(Silent::ConfigGet).await);
    let (outcome, elapsed) = timed("tour state", control.tour_state()).await;
    assert_window("tour state", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_tour_clear_times_out() {
    let control = control(&fake(Silent::ConfigGet).await);
    let (outcome, elapsed) = timed("tour clear", control.clear_tour(3)).await;
    assert_window("tour clear", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_ptz_move_times_out() {
    let control = control(&fake(Silent::PtzControl).await);
    let (outcome, elapsed) = timed(
        "ptz move",
        control.move_ptz(Lens::Bottom, 0.5, 0.0, 0.0, Duration::from_millis(100)),
    )
    .await;
    assert_window("ptz move", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_white_light_read_times_out() {
    let control = control(&fake(Silent::ConfigGet).await);
    let (outcome, elapsed) = timed("white light", control.white_light(None)).await;
    assert_window("white light", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_picture_flip_read_times_out() {
    let control = control(&fake(Silent::ConfigGet).await);
    let (outcome, elapsed) = timed("picture flip", control.picture_flip(None, None)).await;
    assert_window("picture flip", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_record_mode_read_times_out() {
    let control = control(&fake(Silent::ConfigGet).await);
    let (outcome, elapsed) = timed("record mode", control.record_mode(None)).await;
    assert_window("record mode", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_snapshot_times_out() {
    let control = control(&fake(Silent::Snap).await);
    let (outcome, elapsed) = timed("snapshot", control.snapshot()).await;
    assert_window("snapshot", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_logout_keeps_the_answer_the_device_already_gave() {
    let control = control(&fake(Silent::Logout).await);
    let (outcome, elapsed) = timed("snapshot with a silent logout", control.snapshot()).await;
    let picture = outcome.expect("the answer that arrived before the logout is kept");
    assert_eq!(picture, vec![0xff, 0xd8, 0xff, 0x00, 0x01]);
    assert!(
        elapsed < OUTER,
        "the silent logout held the answer for {elapsed:?}"
    );
}
