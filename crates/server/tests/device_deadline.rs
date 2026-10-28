use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proview_core::dvrip::frame::Header;
use proview_core::dvrip::{msg, HEADER_LEN};
use proview_core::provider::Mode;
use proview_server::config::ModeStore;
use rocket::http::{ContentType, Status};
use rocket::local::asynchronous::Client;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const WINDOW_MS: &str = "300";
const OUTER: Duration = Duration::from_secs(8);
const ACK_DELAY: Duration = Duration::from_millis(700);
const PTZ_START: i64 = 65535;
const PTZ_STOP: i64 = -1;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Everything,
    SilentAfterLogin,
    SlowStartAck,
}

#[derive(Clone)]
struct PtzFrame {
    preset: i64,
    step: u8,
    command: String,
    at: Instant,
}

fn data_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pvw-device-deadline-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("PROVIEW_DATA_DIR", &dir);
    dir
}

fn figment() -> rocket::figment::Figment {
    rocket::Config::figment()
        .merge(("secret_key", "uVo3haa/znSxx4MPMV95m4oe9EYQe99QbYgEuBtVqqU="))
        .merge(("port", 0))
}

fn config(device: &str) -> proview_core::config::Config {
    proview_core::config::Config {
        provider: proview_core::config::ProviderKind::Fixture,
        cloud_base: "https://pub-token.xmeye.net".to_string(),
        share_base: proview_core::cloud::attributes::SHARE_BASE.to_string(),
        device_user: "admin".to_string(),
        device_pass: "device-password".to_string(),
        fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
        device_host: Some(device.to_string()),
        device_onvif: None,
        device_rtsp: None,
        session_ttl_secs: 3600,
    }
}

async fn fake(answer: Answer) -> String {
    fake_with_frames(answer).await.0
}

async fn fake_with_frames(answer: Answer) -> (String, Arc<Mutex<Vec<PtzFrame>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let frames = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&frames);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(serve(socket, answer, Arc::clone(&log)));
        }
    });
    (address, frames)
}

fn ptz_frame_of(body: &[u8]) -> Option<PtzFrame> {
    let mut end = body.len();
    while end > 0 && matches!(body[end - 1], 0 | b'\n' | b'\r') {
        end -= 1;
    }
    let value = serde_json::from_slice::<Value>(&body[..end]).ok()?;
    let control = value.get("OPPTZControl")?;
    let parameter = control.get("Parameter")?;
    Some(PtzFrame {
        preset: parameter.get("Preset")?.as_i64()?,
        step: parameter.get("Step")?.as_u64()? as u8,
        command: control.get("Command")?.as_str()?.to_string(),
        at: Instant::now(),
    })
}

async fn wait_for_frame(
    frames: &Arc<Mutex<Vec<PtzFrame>>>,
    preset: i64,
    limit: Duration,
) -> PtzFrame {
    let deadline = Instant::now() + limit;
    loop {
        let found = frames
            .lock()
            .unwrap()
            .iter()
            .find(|frame| frame.preset == preset)
            .cloned();
        if let Some(frame) = found {
            return frame;
        }
        assert!(
            Instant::now() < deadline,
            "no frame with Preset {preset} arrived inside {limit:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
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

fn talking_reply(header: &Header, name: Option<&str>) -> Vec<u8> {
    match (header.id, name) {
        (msg::SNAP, _) => frame(header, &[0xff, 0xd8, 0xff, 0x00, 0x01]),
        (msg::LOGIN, _) => reply(header, &json!({ "Ret": 100, "SessionID": "0x11223344" })),
        (msg::ABILITY, _) => reply(header, &json!({ "Ret": 100, "OtherFunction": {} })),
        (msg::CONFIG_GET, Some("Camera")) => reply(
            header,
            &json!({
                "Ret": 100,
                "Camera": {
                    "Param": [{ "PictureFlip": "0", "PictureMirror": "0" }],
                    "WhiteLight": { "WorkMode": "Off" },
                },
            }),
        ),
        (msg::CONFIG_GET, Some("Record")) => reply(
            header,
            &json!({ "Ret": 100, "Record": [{ "RecordMode": "ConfigRecord" }] }),
        ),
        (msg::CONFIG_GET, Some("Uart")) => reply(header, &json!({ "Ret": 100, "Uart": [] })),
        (msg::CONFIG_GET, Some("Ptz")) => reply(header, &json!({ "Ret": 100, "Ptz": [{}] })),
        _ => reply(header, &json!({ "Ret": 100 })),
    }
}

async fn serve(mut socket: TcpStream, answer: Answer, frames: Arc<Mutex<Vec<PtzFrame>>>) {
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
        let name = requested_name(&body);
        let mut delay = Duration::ZERO;
        if header.id == msg::PTZ_CONTROL {
            if let Some(frame) = ptz_frame_of(&body) {
                if answer == Answer::SlowStartAck && frame.preset == PTZ_START {
                    delay = ACK_DELAY;
                }
                frames.lock().unwrap().push(frame);
            }
        }
        let out = match answer {
            Answer::Everything | Answer::SlowStartAck => talking_reply(&header, name.as_deref()),
            Answer::SilentAfterLogin if header.id == msg::LOGIN => {
                reply(&header, &json!({ "Ret": 100, "SessionID": "0x11223344" }))
            }
            Answer::SilentAfterLogin => {
                std::future::pending::<()>().await;
                return;
            }
        };
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if socket.write_all(&out).await.is_err() {
            return;
        }
    }
}

async fn client(device: &str) -> Client {
    data_dir();
    std::env::set_var("PROVIEW_DVRIP_WINDOW_MS", WINDOW_MS);
    let client = Client::tracked(proview_server::build_with_modes(
        figment(),
        config(device),
        ModeStore::new(Mode::Cloud, None),
    ))
    .await
    .expect("valid rocket instance");
    let status = client
        .post("/api/login")
        .header(ContentType::JSON)
        .body(
            json!({ "email": "fixture@example.invalid", "password": "fixture-password" })
                .to_string(),
        )
        .dispatch()
        .await
        .status();
    assert_eq!(status, Status::Ok);
    client
}

async fn get(client: &Client, path: &str) -> (Status, String) {
    let response = tokio::time::timeout(OUTER, client.get(path).dispatch())
        .await
        .unwrap_or_else(|_| panic!("{path} never answered inside {OUTER:?}"));
    let status = response.status();
    (status, response.into_string().await.unwrap_or_default())
}

const READS: [&str; 5] = [
    "/api/device/controls/light",
    "/api/device/controls/flip",
    "/api/device/controls/record",
    "/api/device/controls/snapshot",
    "/api/device/controls/tour",
];

#[tokio::test]
async fn the_control_reads_answer_from_a_device_that_keeps_talking() {
    let client = client(&fake(Answer::Everything).await).await;
    for (path, expected) in [
        ("/api/device/controls/light", "\"mode\":\"Off\""),
        ("/api/device/controls/flip", "\"flip\":\"0\""),
        ("/api/device/controls/record", "\"mode\":\"ConfigRecord\""),
        ("/api/device/controls/tour", "\"backend\":\"dvrip\""),
    ] {
        let (status, body) = get(&client, path).await;
        assert_eq!(status, Status::Ok, "{path}: {body}");
        assert!(body.contains(expected), "{path}: {body}");
    }
    let (status, _) = get(&client, "/api/device/controls/snapshot").await;
    assert_eq!(status, Status::Ok);
}

#[tokio::test]
async fn a_silent_device_times_every_control_read_out() {
    let client = client(&fake(Answer::SilentAfterLogin).await).await;
    for path in READS {
        let (status, body) = get(&client, path).await;
        assert_eq!(status, Status::GatewayTimeout, "{path}: {body}");
        assert!(body.contains("did not answer inside"), "{path}: {body}");
    }
}

#[tokio::test]
async fn a_ptz_move_whose_start_ack_outlives_the_window_still_stops_the_motor() {
    let (address, frames) = fake_with_frames(Answer::SlowStartAck).await;
    let client = client(&address).await;
    let path = "/api/device/controls/ptz?lens=bottom&pan=0.5&hold_ms=100";
    let started = Instant::now();
    let response = tokio::time::timeout(OUTER, client.post(path).dispatch())
        .await
        .unwrap_or_else(|_| panic!("{path} never answered inside {OUTER:?}"));
    let answered = Instant::now();
    let status = response.status();
    let body = response.into_string().await.unwrap_or_default();
    assert_eq!(status, Status::GatewayTimeout, "{path}: {body}");
    assert!(body.contains("did not answer inside"), "{path}: {body}");

    let start = frames
        .lock()
        .unwrap()
        .iter()
        .find(|frame| frame.preset == PTZ_START)
        .cloned()
        .expect("the start frame reached the camera");
    let stop = wait_for_frame(&frames, PTZ_STOP, Duration::from_secs(3)).await;
    assert_eq!(
        stop.step, start.step,
        "the stop has to repeat the step the start used"
    );
    assert_eq!(
        stop.command, start.command,
        "the stop has to repeat the direction the start used"
    );
    assert!(
        answered - started < Duration::from_millis(600),
        "the timeout path paid for a logout: the 504 took {:?}",
        answered - started
    );
    println!(
        "start ack delayed {ACK_DELAY:?}, 504 after {:?}, stop frame {:?} after the 504 and {:?} after the start",
        answered - started,
        stop.at - answered,
        stop.at - start.at
    );
}

#[tokio::test]
async fn a_silent_device_times_the_ptz_move_out() {
    let client = client(&fake(Answer::SilentAfterLogin).await).await;
    let path = "/api/device/controls/ptz?lens=bottom&pan=0.5&hold_ms=100";
    let response = tokio::time::timeout(OUTER, client.post(path).dispatch())
        .await
        .unwrap_or_else(|_| panic!("{path} never answered inside {OUTER:?}"));
    let status = response.status();
    let body = response.into_string().await.unwrap_or_default();
    assert_eq!(status, Status::GatewayTimeout, "{path}: {body}");
    assert!(body.contains("did not answer inside"), "{path}: {body}");
}
