use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use proview_core::config::{Config, ProviderKind};
use proview_core::device::{ControlError, DeviceControl, DvripControl, Lens};
use proview_core::dvrip::frame::Header;
use proview_core::dvrip::{msg, HEADER_LEN};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const WINDOW: Duration = Duration::from_millis(300);
const OUTER: Duration = Duration::from_secs(4);
const HOLD: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Silent {
    None,
    Ability,
    ConfigGet,
    PtzControl,
    Snap,
    Logout,
}

impl Silent {
    fn matches(self, id: u16) -> bool {
        match self {
            Silent::None => false,
            Silent::Ability => id == msg::ABILITY,
            Silent::ConfigGet => id == msg::CONFIG_GET,
            Silent::PtzControl => id == msg::PTZ_CONTROL,
            Silent::Snap => id == msg::SNAP,
            Silent::Logout => id == msg::LOGOUT,
        }
    }
}

#[derive(Clone, Copy)]
struct Behaviour {
    silent: Silent,
    listing_delay: Duration,
    write_ret: i64,
}

impl Default for Behaviour {
    fn default() -> Self {
        Self {
            silent: Silent::None,
            listing_delay: Duration::ZERO,
            write_ret: 100,
        }
    }
}

struct Fake {
    address: String,
    ptz_frames: Arc<AtomicUsize>,
}

fn config(device: &str) -> Config {
    Config {
        provider: ProviderKind::Fixture,
        cloud_base: "https://pub-token.xmeye.net".to_string(),
        device_user: "admin".to_string(),
        device_pass: "device-password".to_string(),
        fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
        device_host: Some(device.to_string()),
        device_onvif: None,
        device_rtsp: None,
        session_ttl_secs: 3600,
    }
}

fn control(device: &str, window: Duration) -> DvripControl {
    DvripControl::with_window(&config(device), window).expect("the device host is set")
}

async fn fake(behaviour: Behaviour) -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let ptz_frames = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&ptz_frames);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(serve(socket, behaviour, Arc::clone(&counter)));
        }
    });
    Fake {
        address,
        ptz_frames,
    }
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

async fn serve(mut socket: TcpStream, behaviour: Behaviour, ptz_frames: Arc<AtomicUsize>) {
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
        if behaviour.silent.matches(header.id) {
            std::future::pending::<()>().await;
            return;
        }
        let name = requested_name(&body);
        let mut delay = Duration::ZERO;
        let out = match (header.id, name.as_deref()) {
            (msg::LOGIN, _) => reply(&header, &json!({ "Ret": 100, "SessionID": "0x11223344" })),
            (msg::SNAP, _) => frame(&header, &[0xff, 0xd8, 0xff, 0x00, 0x01]),
            (msg::PTZ_CONTROL, _) => {
                ptz_frames.fetch_add(1, Ordering::SeqCst);
                reply(&header, &json!({ "Ret": behaviour.write_ret }))
            }
            (msg::CONFIG_GET, Some("Uart")) => {
                delay = behaviour.listing_delay;
                reply(&header, &json!({ "Ret": 100, "Uart": [] }))
            }
            (msg::CONFIG_GET, Some(name)) => reply(&header, &json!({ "Ret": 100, name: [{}] })),
            _ => reply(&header, &json!({ "Ret": 100 })),
        };
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
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
    let device = fake(Behaviour {
        silent: Silent::Ability,
        ..Behaviour::default()
    })
    .await;
    let control = control(&device.address, WINDOW);
    let (outcome, elapsed) = timed("capabilities", control.capabilities()).await;
    assert_window("capabilities", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_tour_listing_times_out() {
    let device = fake(Behaviour {
        silent: Silent::ConfigGet,
        ..Behaviour::default()
    })
    .await;
    let control = control(&device.address, WINDOW);
    let (outcome, elapsed) = timed("tour state", control.tour_state()).await;
    assert_window("tour state", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_tour_clear_times_out() {
    let device = fake(Behaviour {
        silent: Silent::ConfigGet,
        ..Behaviour::default()
    })
    .await;
    let control = control(&device.address, WINDOW);
    let (outcome, elapsed) = timed("tour clear", control.clear_tour(3)).await;
    assert_window("tour clear", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_ptz_move_times_out() {
    let device = fake(Behaviour {
        silent: Silent::PtzControl,
        ..Behaviour::default()
    })
    .await;
    let control = control(&device.address, WINDOW);
    let (outcome, elapsed) = timed(
        "ptz move",
        control.move_ptz(Lens::Bottom, 0.5, 0.0, 0.0, Duration::from_millis(100)),
    )
    .await;
    assert_window("ptz move", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_white_light_read_times_out() {
    let device = fake(Behaviour {
        silent: Silent::ConfigGet,
        ..Behaviour::default()
    })
    .await;
    let control = control(&device.address, WINDOW);
    let (outcome, elapsed) = timed("white light", control.white_light(None)).await;
    assert_window("white light", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_picture_flip_read_times_out() {
    let device = fake(Behaviour {
        silent: Silent::ConfigGet,
        ..Behaviour::default()
    })
    .await;
    let control = control(&device.address, WINDOW);
    let (outcome, elapsed) = timed("picture flip", control.picture_flip(None, None)).await;
    assert_window("picture flip", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_record_mode_read_times_out() {
    let device = fake(Behaviour {
        silent: Silent::ConfigGet,
        ..Behaviour::default()
    })
    .await;
    let control = control(&device.address, WINDOW);
    let (outcome, elapsed) = timed("record mode", control.record_mode(None)).await;
    assert_window("record mode", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_snapshot_times_out() {
    let device = fake(Behaviour {
        silent: Silent::Snap,
        ..Behaviour::default()
    })
    .await;
    let control = control(&device.address, WINDOW);
    let (outcome, elapsed) = timed("snapshot", control.snapshot()).await;
    assert_window("snapshot", outcome, elapsed);
}

#[tokio::test]
async fn a_silent_logout_keeps_the_answer_the_device_already_gave() {
    let device = fake(Behaviour {
        silent: Silent::Logout,
        ..Behaviour::default()
    })
    .await;
    let control = control(&device.address, WINDOW);
    let (outcome, elapsed) = timed("snapshot with a silent logout", control.snapshot()).await;
    let picture = outcome.expect("the answer that arrived before the logout is kept");
    assert_eq!(picture, vec![0xff, 0xd8, 0xff, 0x00, 0x01]);
    assert!(
        elapsed < OUTER,
        "the silent logout held the answer for {elapsed:?}"
    );
}

#[tokio::test]
async fn a_ptz_move_whose_hold_outlives_the_window_still_stops_the_motor() {
    let device = fake(Behaviour::default()).await;
    let control = control(&device.address, WINDOW);
    let (outcome, elapsed) = timed(
        "ptz move with a hold past the window",
        control.move_ptz(Lens::Bottom, 0.5, 0.0, 0.0, HOLD),
    )
    .await;
    assert!(
        outcome.is_ok(),
        "a hold longer than the window kept the move from finishing: {:?}",
        outcome.err()
    );
    assert_eq!(
        device.ptz_frames.load(Ordering::SeqCst),
        2,
        "the start and the stop both have to reach the camera"
    );
    assert!(
        elapsed >= HOLD && elapsed < OUTER,
        "the move took {elapsed:?}, expected at least {HOLD:?}"
    );
}

#[tokio::test]
async fn a_tour_write_fails_closed_when_its_listing_times_out() {
    let device = fake(Behaviour {
        listing_delay: Duration::from_millis(450),
        write_ret: 103,
        ..Behaviour::default()
    })
    .await;
    let control = control(&device.address, WINDOW);
    let (outcome, elapsed) = timed("tour clear with a late listing", control.clear_tour(3)).await;
    assert_window("tour clear with a late listing", outcome, elapsed);
    assert_eq!(
        device.ptz_frames.load(Ordering::SeqCst),
        0,
        "a write must not go out on channel 0 after its listing timed out"
    );
}
