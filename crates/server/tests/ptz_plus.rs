use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proview_core::dvrip::frame::Header;
use proview_core::dvrip::{msg, HEADER_LEN};
use proview_core::provider::Mode;
use proview_core::provider::VendorSession;
use proview_core::session::Store;
use proview_server::config::ModeStore;
use rocket::http::{ContentType, Cookie, Status};
use rocket::local::asynchronous::Client;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const WINDOW_MS: u64 = 400;
const HOLD_MS: u64 = 2500;
const SLOW_ABILITY_MS: u64 = 350;
const OUTER: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mood {
    Answers,
    SilentOnPtz,
    SilentOnLocate,
    NoPanorama,
    SlowAbility,
    DropsAfterStop,
    StallsParkedThenRefusesLogin,
}

#[derive(Clone)]
struct Seen {
    connection: usize,
    at: Instant,
    id: u16,
    body: Value,
}

#[derive(Clone)]
struct Device {
    mood: Mood,
    seen: Arc<Mutex<Vec<Seen>>>,
    presets: Arc<Mutex<Vec<(i64, String)>>>,
    tour: Arc<Mutex<Vec<i64>>>,
    connections: Arc<Mutex<usize>>,
    dropped: Option<&'static str>,
    mirrored: bool,
}

const ABILITIES: [&str; 5] = [
    "SupportPTZTour",
    "SupportBallTelescopic",
    "SupportGunBallTwoSensorPtzLocate",
    "SupportPtzAutoAdjust",
    "SupportPTZDirectionControl",
];

impl Device {
    fn without(mut self, ability: &'static str) -> Self {
        self.dropped = Some(ability);
        self
    }

    fn mirrored(mut self) -> Self {
        self.mirrored = true;
        self
    }

    fn system_function(&self) -> Value {
        let mut other = serde_json::Map::new();
        for ability in ABILITIES {
            if Some(ability) != self.dropped {
                other.insert(ability.to_string(), json!(true));
            }
        }
        json!({ "Ret": 100, "SystemFunction": { "OtherFunction": other } })
    }

    fn writes(&self) -> Vec<Seen> {
        self.frames()
            .into_iter()
            .filter(|frame| {
                matches!(
                    frame.id,
                    msg::PTZ_CONTROL | msg::PTZ_LOCATE | msg::OP_MACHINE
                )
            })
            .collect()
    }

    fn frames(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn ptz(&self) -> Vec<Seen> {
        self.frames()
            .into_iter()
            .filter(|frame| frame.id == msg::PTZ_CONTROL)
            .collect()
    }

    fn uart(&self) -> Value {
        let presets: Vec<Value> = self
            .presets
            .lock()
            .unwrap()
            .iter()
            .map(|(id, name)| json!({ "Id": id, "PresetName": name }))
            .collect();
        let points: Vec<Value> = self
            .tour
            .lock()
            .unwrap()
            .iter()
            .map(|preset| json!({ "Id": preset, "Time": 5 }))
            .collect();
        json!({
            "PTZPreset": [presets],
            "PTZTour": [[{ "Id": 1, "Name": "Yard", "Tour": points }]],
        })
    }

    fn apply(&self, body: &Value) {
        let control = &body["OPPTZControl"];
        let parameter = &control["Parameter"];
        let preset = parameter["Preset"].as_i64().unwrap_or(0);
        let name = parameter["PresetName"].as_str().unwrap_or("").to_string();
        match control["Command"].as_str().unwrap_or("") {
            "SetPreset" => self.presets.lock().unwrap().push((preset, name)),
            "SetPresetName" => {
                for entry in self.presets.lock().unwrap().iter_mut() {
                    if entry.0 == preset {
                        entry.1 = name.clone();
                    }
                }
            }
            "ClearPreset" => self
                .presets
                .lock()
                .unwrap()
                .retain(|entry| entry.0 != preset),
            "DeleteTour" => self.tour.lock().unwrap().retain(|stop| *stop != preset),
            _ => {}
        }
    }
}

fn device(mood: Mood, presets: &[(i64, &str)], tour: &[i64]) -> Device {
    Device {
        mood,
        seen: Arc::new(Mutex::new(Vec::new())),
        presets: Arc::new(Mutex::new(
            presets
                .iter()
                .map(|(id, name)| (*id, name.to_string()))
                .collect(),
        )),
        tour: Arc::new(Mutex::new(tour.to_vec())),
        connections: Arc::new(Mutex::new(0)),
        dropped: None,
        mirrored: false,
    }
}

async fn listen(device: Device) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let connection = {
                let mut count = device.connections.lock().unwrap();
                *count += 1;
                *count
            };
            tokio::spawn(serve(socket, device.clone(), connection));
        }
    });
    address
}

fn trim(payload: &[u8]) -> &[u8] {
    let mut end = payload.len();
    while end > 0 && matches!(payload[end - 1], 0 | b'\n' | b'\r') {
        end -= 1;
    }
    &payload[..end]
}

fn reply(header: &Header, value: &Value) -> Vec<u8> {
    let mut payload = value.to_string().into_bytes();
    payload.push(b'\n');
    payload.push(0);
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
    out.extend_from_slice(&payload);
    out
}

async fn serve(mut socket: TcpStream, device: Device, connection: usize) {
    let mut stopped = false;
    loop {
        let mut header_bytes = [0u8; HEADER_LEN];
        if socket.read_exact(&mut header_bytes).await.is_err() {
            return;
        }
        let Ok(header) = Header::decode(&header_bytes) else {
            return;
        };
        let mut raw = vec![0u8; header.length as usize];
        if header.length > 0 && socket.read_exact(&mut raw).await.is_err() {
            return;
        }
        let body = serde_json::from_slice::<Value>(trim(&raw)).unwrap_or(Value::Null);
        device.seen.lock().unwrap().push(Seen {
            connection,
            at: Instant::now(),
            id: header.id,
            body: body.clone(),
        });
        let stalls = device.mood == Mood::StallsParkedThenRefusesLogin;
        let answer = match header.id {
            msg::LOGIN if stalls && connection == 2 => Some(json!({ "Ret": 203 })),
            msg::LOGIN => Some(json!({ "Ret": 100, "SessionID": "0x11223344" })),
            msg::PTZ_CONTROL if stalls && stopped => None,
            msg::CONFIG_GET if body["Name"] == "Uart.PTZControlCmd" && device.mirrored => {
                Some(json!({
                    "Ret": 100,
                    "Name": "Uart.PTZControlCmd",
                    "Uart.PTZControlCmd": [{ "FlipOperation": false, "MirrorOperation": true, "ModifyCfg": true }]
                }))
            }
            msg::CONFIG_GET if body["Name"] == "Uart" => {
                Some(json!({ "Ret": 100, "Uart": device.uart() }))
            }
            msg::PTZ_CONTROL if device.mood == Mood::SilentOnPtz => None,
            msg::PTZ_CONTROL => {
                device.apply(&body);
                Some(json!({ "Ret": 100 }))
            }
            msg::PTZ_LOCATE if device.mood == Mood::SilentOnLocate => None,
            msg::ABILITY if body["Name"] == "SystemFunction" => {
                if device.mood == Mood::SlowAbility {
                    tokio::time::sleep(Duration::from_millis(SLOW_ABILITY_MS)).await;
                }
                Some(device.system_function())
            }
            msg::ABILITY if body["Name"] == "GoToPosition" && device.mood == Mood::NoPanorama => {
                Some(json!({ "Ret": 103 }))
            }
            msg::ABILITY if body["Name"] == "GoToPosition" => Some(json!({
                "Ret": 100,
                "GoToPosition": { "HorStart": 0, "HorSteps": 3600, "HorNow": 100, "VerStart": 45, "VerSteps": 90, "VerNow": 45 }
            })),
            _ => Some(json!({ "Ret": 100 })),
        };
        let Some(value) = answer else {
            continue;
        };
        if socket.write_all(&reply(&header, &value)).await.is_err() {
            return;
        }
        let stop =
            header.id == msg::PTZ_CONTROL && body["OPPTZControl"]["Parameter"]["Preset"] == -1;
        if device.mood == Mood::DropsAfterStop && stop {
            return;
        }
        stopped |= stop;
    }
}

fn figment() -> rocket::figment::Figment {
    rocket::Config::figment()
        .merge(("secret_key", "uVo3haa/znSxx4MPMV95m4oe9EYQe99QbYgEuBtVqqU="))
        .merge(("port", 0))
}

fn config(address: &str) -> proview_core::config::Config {
    proview_core::config::Config {
        provider: proview_core::config::ProviderKind::Fixture,
        cloud_base: "https://pub-token.xmeye.net".to_string(),
        share_base: proview_core::cloud::attributes::SHARE_BASE.to_string(),
        device_user: "admin".to_string(),
        device_pass: "device-password".to_string(),
        fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
        device_host: Some(address.to_string()),
        device_onvif: None,
        device_rtsp: None,
        session_ttl_secs: 3600,
    }
}

async fn client(address: &str) -> Client {
    std::env::set_var("PROVIEW_DVRIP_WINDOW_MS", WINDOW_MS.to_string());
    std::env::set_var("PROVIEW_PTZ_HOLD_MS", HOLD_MS.to_string());
    let client = Client::tracked(proview_server::build_with_modes(
        figment(),
        config(address),
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

async fn post(client: &Client, path: &str) -> (Status, Value, Duration) {
    let started = Instant::now();
    let response = tokio::time::timeout(OUTER, client.post(path).dispatch())
        .await
        .unwrap_or_else(|_| panic!("{path} never answered inside {OUTER:?}"));
    let status = response.status();
    let text = response.into_string().await.unwrap_or_default();
    (
        status,
        serde_json::from_str(&text).unwrap_or(json!({ "raw": text })),
        started.elapsed(),
    )
}

fn command(frame: &Seen) -> String {
    frame.body["OPPTZControl"]["Command"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn preset_of(frame: &Seen) -> i64 {
    frame.body["OPPTZControl"]["Parameter"]["Preset"]
        .as_i64()
        .unwrap_or(0)
}

#[tokio::test]
async fn a_held_move_starts_on_press_and_stops_on_release_over_one_session() {
    let fake = device(Mood::Answers, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    let (status, body, _) = post(&client, "/api/device/controls/ptz/press?action=left").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["command"], "DirectionLeft");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        fake.ptz().len(),
        1,
        "no stop may go out while the key is held"
    );
    let (status, body, _) = post(&client, "/api/device/controls/ptz/release?pressed_ms=2000").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["held"], true);
    assert_eq!(body["times"]["fresh_session"], false);
    let frames = fake.ptz();
    assert_eq!(frames.len(), 2);
    assert_eq!(command(&frames[0]), "DirectionLeft");
    assert_eq!(command(&frames[1]), "DirectionLeft");
    assert_eq!(preset_of(&frames[0]), 65535);
    assert_eq!(preset_of(&frames[1]), -1);
    assert_eq!(
        frames[0].body["OPPTZControl"]["Parameter"]["Step"],
        frames[1].body["OPPTZControl"]["Parameter"]["Step"]
    );
    assert_eq!(frames[0].connection, frames[1].connection);
    let (status, body, _) = post(&client, "/api/device/controls/ptz/release").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["held"], false);
    assert_eq!(fake.ptz().len(), 2);
}

#[tokio::test]
async fn a_tap_stops_as_soon_as_the_press_is_acked_like_the_app_pad() {
    let fake = device(Mood::Answers, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    let (status, _, _) = post(&client, "/api/device/controls/ptz/press?action=left").await;
    assert_eq!(status, Status::Ok);
    let (status, body, _) = post(&client, "/api/device/controls/ptz/release?pressed_ms=100").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["times"]["held_ms"], 100);
    let frames = fake.ptz();
    assert_eq!(preset_of(&frames[0]), 65535);
    assert_eq!(preset_of(&frames[1]), -1);
    let ran = frames[1].at - frames[0].at;
    assert!(
        ran < Duration::from_millis(300),
        "PtzView ACTION_UP sends the stop at once, but it came {ran:?} after the start"
    );
}

#[tokio::test]
async fn the_next_press_reuses_the_parked_session_instead_of_logging_in_again() {
    let fake = device(Mood::Answers, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    let (status, body, _) = post(&client, "/api/device/controls/ptz/press?action=left").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["reused_session"], false);
    let _ = post(&client, "/api/device/controls/ptz/release?pressed_ms=900").await;
    let (status, body, _) = post(&client, "/api/device/controls/ptz/press?action=right").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["reused_session"], true);
    assert_eq!(body["session_ms"], 0);
    let _ = post(&client, "/api/device/controls/ptz/release?pressed_ms=900").await;
    let frames = fake.ptz();
    assert_eq!(frames.len(), 4);
    assert!(frames
        .iter()
        .all(|frame| frame.connection == frames[0].connection));
    let logins = fake
        .frames()
        .iter()
        .filter(|frame| frame.id == msg::LOGIN)
        .count();
    assert_eq!(logins, 1);
}

async fn second_account(address: &str) -> (Client, Cookie<'static>) {
    let client = Client::untracked(proview_server::build_with_modes(
        figment(),
        config(address),
        ModeStore::new(Mode::Cloud, None),
    ))
    .await
    .expect("valid rocket instance");
    client
        .rocket()
        .state::<Store>()
        .expect("session store")
        .put(
            "account-b".to_string(),
            VendorSession {
                account: "other@example.invalid".to_string(),
                token: None,
            },
            Duration::from_secs(3600),
        );
    (client, Cookie::new("pvw_session", "account-b"))
}

async fn post_as(client: &Client, cookie: &Cookie<'static>, path: &str) -> (Status, Value) {
    let response = tokio::time::timeout(
        OUTER,
        client
            .post(path.to_string())
            .private_cookie(cookie.clone())
            .dispatch(),
    )
    .await
    .unwrap_or_else(|_| panic!("{path} never answered inside {OUTER:?}"));
    let status = response.status();
    let text = response.into_string().await.unwrap_or_default();
    (
        status,
        serde_json::from_str(&text).unwrap_or(json!({ "raw": text })),
    )
}

#[tokio::test]
async fn another_account_never_takes_the_session_a_stop_parked() {
    let fake = device(Mood::Answers, &[], &[]);
    let address = listen(fake.clone()).await;
    let owner = client(&address).await;
    let (status, _, _) = post(&owner, "/api/device/controls/ptz/press?action=left").await;
    assert_eq!(status, Status::Ok);
    let _ = post(&owner, "/api/device/controls/ptz/release?pressed_ms=900").await;
    let (other, cookie) = second_account(&address).await;
    let (status, body) = post_as(
        &other,
        &cookie,
        "/api/device/controls/ptz/press?action=right",
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(
        body["reused_session"], false,
        "account B rode account A's parked login: {body}"
    );
    let _ = post_as(
        &other,
        &cookie,
        "/api/device/controls/ptz/release?pressed_ms=900",
    )
    .await;
    let frames = fake.ptz();
    assert_ne!(frames[2].connection, frames[0].connection);
    let (status, body, _) = post(&owner, "/api/device/controls/ptz/press?action=up").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["reused_session"], true, "{body}");
    let _ = post(&owner, "/api/device/controls/ptz/release?pressed_ms=900").await;
    assert_eq!(fake.ptz()[4].connection, frames[0].connection);
}

#[tokio::test]
async fn a_parked_press_that_times_out_still_gets_a_stop_when_the_fresh_login_fails() {
    let fake = device(Mood::StallsParkedThenRefusesLogin, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    let (status, _, _) = post(&client, "/api/device/controls/ptz/press?action=left").await;
    assert_eq!(status, Status::Ok);
    let _ = post(&client, "/api/device/controls/ptz/release?pressed_ms=900").await;
    let (status, body, _) = post(&client, "/api/device/controls/ptz/press?action=right").await;
    assert_ne!(status, Status::Ok, "{body}");
    let deadline = Instant::now() + Duration::from_secs(4);
    let stop_after_stall = || {
        fake.ptz().iter().any(|frame| {
            frame.connection >= 3 && preset_of(frame) == -1 && command(frame) == "DirectionRight"
        })
    };
    while Instant::now() < deadline && !stop_after_stall() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        stop_after_stall(),
        "the stalled parked press never got its stop"
    );
}

#[tokio::test]
async fn a_parked_session_the_camera_dropped_falls_back_to_a_fresh_login() {
    let fake = device(Mood::DropsAfterStop, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    let (status, _, _) = post(&client, "/api/device/controls/ptz/press?action=left").await;
    assert_eq!(status, Status::Ok);
    let _ = post(&client, "/api/device/controls/ptz/release?pressed_ms=900").await;
    let (status, body, took) = post(&client, "/api/device/controls/ptz/press?action=right").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["reused_session"], false);
    assert!(
        took < Duration::from_millis(1500),
        "the fallback took {took:?}"
    );
    let (status, body, _) = post(&client, "/api/device/controls/ptz/release?pressed_ms=900").await;
    assert_eq!(status, Status::Ok, "{body}");
    let frames = fake.ptz();
    let last = frames.last().unwrap();
    assert_eq!(command(last), "DirectionRight");
    assert_eq!(preset_of(last), -1);
    assert_ne!(last.connection, frames[0].connection);
}

#[tokio::test]
async fn a_mirrored_camera_gets_direction_right_for_the_left_key_with_the_app_steps() {
    let fake = device(Mood::Answers, &[], &[]).mirrored();
    let client = client(&listen(fake.clone()).await).await;
    let (status, body, _) = post(&client, "/api/device/controls/ptz/press?action=left").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["action"], "left");
    assert_eq!(body["command"], "DirectionRight");
    assert_eq!(body["speed"], "normal");
    assert_eq!(body["step"], 4);
    assert_eq!(body["orientation"]["mirror"], true);
    let (status, _, _) = post(&client, "/api/device/controls/ptz/release?pressed_ms=900").await;
    assert_eq!(status, Status::Ok);
    let (status, body, _) = post(
        &client,
        "/api/device/controls/ptz/press?action=up&speed=slow",
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["command"], "DirectionUp");
    assert_eq!(body["step"], 1);
    let (status, _, _) = post(&client, "/api/device/controls/ptz/release?pressed_ms=900").await;
    assert_eq!(status, Status::Ok);
    let frames: Vec<(String, i64, i64)> = fake
        .ptz()
        .iter()
        .map(|frame| {
            (
                command(frame),
                preset_of(frame),
                frame.body["OPPTZControl"]["Parameter"]["Step"]
                    .as_i64()
                    .unwrap_or(0),
            )
        })
        .collect();
    assert_eq!(
        frames,
        [
            ("DirectionRight".to_string(), 65535, 4),
            ("DirectionRight".to_string(), -1, 4),
            ("DirectionUp".to_string(), 65535, 1),
            ("DirectionUp".to_string(), -1, 1),
        ]
    );
    let reads = fake
        .frames()
        .iter()
        .filter(|frame| frame.id == msg::CONFIG_GET && frame.body["Name"] == "Uart.PTZControlCmd")
        .count();
    assert_eq!(reads, 1, "the orientation is cached after the first press");
    let (status, body, _) = post(
        &client,
        "/api/device/controls/ptz/press?action=left&speed=4",
    )
    .await;
    assert_eq!(status, Status::BadRequest, "{body}");
}

#[tokio::test]
async fn a_second_press_stops_the_first_before_it_starts() {
    let fake = device(Mood::Answers, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    assert_eq!(
        post(&client, "/api/device/controls/ptz/press?action=up")
            .await
            .0,
        Status::Ok
    );
    assert_eq!(
        post(&client, "/api/device/controls/ptz/press?action=right")
            .await
            .0,
        Status::Ok
    );
    let order: Vec<(String, i64)> = fake
        .ptz()
        .iter()
        .map(|frame| (command(frame), preset_of(frame)))
        .collect();
    assert_eq!(
        order,
        [
            ("DirectionUp".to_string(), 65535),
            ("DirectionUp".to_string(), -1),
            ("DirectionRight".to_string(), 65535),
        ]
    );
    assert_eq!(
        post(&client, "/api/device/controls/ptz/release?pressed_ms=900")
            .await
            .0,
        Status::Ok
    );
}

#[tokio::test]
async fn a_press_the_device_never_acks_times_out_and_still_sends_a_stop() {
    let fake = device(Mood::SilentOnPtz, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    let (status, body, took) = post(&client, "/api/device/controls/ptz/press?action=down").await;
    assert_eq!(status, Status::GatewayTimeout, "{body}");
    assert!(body.to_string().contains("ptz press"), "{body}");
    assert!(
        took < Duration::from_millis(WINDOW_MS + 1500),
        "answered after {took:?}"
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && !fake.ptz().iter().any(|frame| preset_of(frame) == -1) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let frames = fake.ptz();
    let stop = frames
        .iter()
        .find(|frame| preset_of(frame) == -1)
        .expect("a stop after the silent press");
    assert_eq!(command(stop), "DirectionDown");
    assert_ne!(stop.connection, frames[0].connection);
}

#[tokio::test]
async fn a_click_on_the_wide_lens_sends_the_offsets_monitor_activity_computes() {
    let fake = device(Mood::Answers, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    let (status, body, _) = post(&client, "/api/device/controls/ptz/locate?x=0.25&y=0.75").await;
    assert_eq!(status, Status::Ok, "{body}");
    let locate: Vec<Seen> = fake
        .frames()
        .into_iter()
        .filter(|frame| frame.id == msg::PTZ_LOCATE)
        .collect();
    assert_eq!(locate.len(), 1);
    assert_eq!(locate[0].body["Name"], "OPPtzLocate");
    assert_eq!(
        locate[0].body["OPPtzLocate"],
        json!([{ "xoffset": -2048, "yoffset": -2048 }])
    );
    let before = fake.frames().len();
    let (status, _, _) = post(&client, "/api/device/controls/ptz/locate?x=1.5&y=0.5").await;
    assert_eq!(status, Status::BadRequest);
    assert_eq!(
        fake.frames().len(),
        before,
        "an outside click must not reach the camera"
    );
}

#[tokio::test]
async fn a_locate_the_device_never_answers_maps_to_504_inside_the_window() {
    let fake = device(Mood::SilentOnLocate, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    let (status, body, took) = post(&client, "/api/device/controls/ptz/locate?x=0.5&y=0.5").await;
    assert_eq!(status, Status::GatewayTimeout, "{body}");
    assert!(body.to_string().contains("ptz locate"), "{body}");
    assert!(
        took < Duration::from_millis(WINDOW_MS + 1500),
        "answered after {took:?}"
    );
}

#[tokio::test]
async fn presets_are_added_at_the_lowest_free_id_renamed_and_deleted_with_their_tour_stops() {
    let fake = device(
        Mood::Answers,
        &[(1, "Gate"), (2, "Drive"), (250, "Home")],
        &[1, 2],
    );
    let client = client(&listen(fake.clone()).await).await;
    let (status, body, _) = post(&client, "/api/device/controls/preset/add").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["id"], 3);
    assert_eq!(body["name"], "Preset3");
    let set = fake
        .ptz()
        .into_iter()
        .find(|frame| command(frame) == "SetPreset")
        .unwrap();
    assert_eq!(
        set.body["OPPTZControl"]["Parameter"]["PresetName"],
        "Preset3"
    );
    assert_eq!(set.body["OPPTZControl"]["Parameter"]["Channel"], 0);

    let (status, body, _) = post(
        &client,
        "/api/device/controls/preset/rename?token=3&name=%20Back%20porch%20by%20the%20shed%20",
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["name"], "Back porch by th");
    assert!(
        body["presets"].to_string().contains("Back porch by th"),
        "{body}"
    );

    let (status, _, _) = post(
        &client,
        "/api/device/controls/preset/rename?token=3&name=%20%20",
    )
    .await;
    assert_eq!(status, Status::BadRequest);
    let (status, _, _) = post(&client, "/api/device/controls/preset/rename?token=9&name=x").await;
    assert_eq!(status, Status::NotFound);

    let (status, body, _) = post(
        &client,
        "/api/device/controls/tour/add?tour=1&preset=3&index=2",
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    fake.tour.lock().unwrap().push(3);
    let (status, body, _) = post(&client, "/api/device/controls/preset/delete?token=3").await;
    assert_eq!(status, Status::Ok, "{body}");
    let commands: Vec<String> = fake.ptz().iter().map(command).collect();
    let clear = commands
        .iter()
        .position(|name| name == "ClearPreset")
        .unwrap();
    assert_eq!(commands[clear + 1], "DeleteTour");
    assert_eq!(*fake.tour.lock().unwrap(), vec![1, 2]);
    let ids: Vec<i64> = fake
        .presets
        .lock()
        .unwrap()
        .iter()
        .map(|entry| entry.0)
        .collect();
    assert_eq!(ids, vec![1, 2, 250]);
    let (status, _, _) = post(&client, "/api/device/controls/preset/delete?token=3").await;
    assert_eq!(status, Status::NotFound);
}

#[tokio::test]
async fn a_full_preset_list_refuses_the_add_before_any_write() {
    let full: Vec<(i64, &str)> = (1..=32).map(|id| (id, "p")).collect();
    let fake = device(Mood::Answers, &full, &[]);
    let client = client(&listen(fake.clone()).await).await;
    let (status, body, _) = post(&client, "/api/device/controls/preset/add?name=x").await;
    assert_eq!(status, Status::Conflict, "{body}");
    assert!(fake.ptz().is_empty());
}

#[tokio::test]
async fn a_tour_stop_is_removed_with_delete_tour_on_its_line() {
    let fake = device(Mood::Answers, &[(1, "Gate"), (2, "Drive")], &[1, 2]);
    let client = client(&listen(fake.clone()).await).await;
    let (status, body, _) = post(&client, "/api/device/controls/tour/remove?tour=1&preset=2").await;
    assert_eq!(status, Status::Ok, "{body}");
    let frame = fake
        .ptz()
        .into_iter()
        .find(|frame| command(frame) == "DeleteTour")
        .unwrap();
    assert_eq!(frame.body["OPPTZControl"]["Parameter"]["Tour"], 1);
    assert_eq!(frame.body["OPPTZControl"]["Parameter"]["Preset"], 2);
    assert_eq!(*fake.tour.lock().unwrap(), vec![1]);
    let (status, _, _) = post(&client, "/api/device/controls/tour/remove?tour=1&preset=7").await;
    assert_eq!(status, Status::NotFound);
}

#[tokio::test]
async fn the_panorama_goes_to_a_column_at_the_vertical_start_inside_the_range() {
    let fake = device(Mood::Answers, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    let (status, body, _) = post(&client, "/api/device/controls/panorama/goto?left=1800").await;
    assert_eq!(status, Status::Ok, "{body}");
    let frame = fake
        .ptz()
        .into_iter()
        .find(|frame| command(frame) == "GoToPosition")
        .expect("a GoToPosition frame");
    assert_eq!(
        frame.body["OPPTZControl"]["Parameter"]["POINT"],
        json!({ "left": 1800, "top": 45 })
    );
    let before = fake.ptz().len();
    let (status, _, _) = post(&client, "/api/device/controls/panorama/goto?left=3700").await;
    assert_eq!(status, Status::BadRequest);
    assert_eq!(fake.ptz().len(), before);
}

#[tokio::test]
async fn a_camera_that_refuses_the_panorama_ability_answers_501_with_its_code() {
    let fake = device(Mood::NoPanorama, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    let response = client.get("/api/device/controls/panorama").dispatch().await;
    assert_eq!(response.status(), Status::NotImplemented);
    let text = response.into_string().await.unwrap_or_default();
    assert!(text.contains("Ret 103"), "{text}");
    let (status, _, _) = post(&client, "/api/device/controls/panorama/goto?left=10").await;
    assert_eq!(status, Status::NotImplemented);
    assert!(fake.ptz().is_empty());
}

#[tokio::test]
async fn calibration_sends_ptz_auto_adjust_on_the_machine_message() {
    let fake = device(Mood::Answers, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    let (status, body, _) = post(&client, "/api/device/controls/ptz/calibrate").await;
    assert_eq!(status, Status::Ok, "{body}");
    let frame = fake
        .frames()
        .into_iter()
        .find(|frame| frame.id == msg::OP_MACHINE)
        .expect("an OPMachine frame");
    assert_eq!(frame.body["Name"], "OPPtzAutoAdjust");
}

const REFUSALS: [(&str, &str); 12] = [
    (
        "SupportBallTelescopic",
        "/api/device/controls/ptz/press?action=zoom-in",
    ),
    (
        "SupportBallTelescopic",
        "/api/device/controls/ptz/press?action=focus-far",
    ),
    (
        "SupportGunBallTwoSensorPtzLocate",
        "/api/device/controls/ptz/locate?x=0.5&y=0.5",
    ),
    ("SupportPTZTour", "/api/device/controls/preset/add?name=x"),
    (
        "SupportPTZTour",
        "/api/device/controls/preset/rename?token=1&name=x",
    ),
    (
        "SupportPTZTour",
        "/api/device/controls/preset/delete?token=1",
    ),
    (
        "SupportPTZTour",
        "/api/device/controls/tour/remove?tour=1&preset=1",
    ),
    (
        "SupportPTZTour",
        "/api/device/controls/tour/add?tour=1&preset=1",
    ),
    ("SupportPTZTour", "/api/device/controls/tour/clear?tour=1"),
    ("SupportPTZTour", "/api/device/controls/tour/start?tour=1"),
    ("SupportPTZTour", "/api/device/controls/tour/stop?tour=1"),
    ("SupportPtzAutoAdjust", "/api/device/controls/ptz/calibrate"),
];

#[tokio::test]
async fn a_write_the_device_does_not_report_answers_501_before_anything_is_sent() {
    for (ability, path) in REFUSALS {
        let fake = device(Mood::Answers, &[(1, "Gate")], &[1]).without(ability);
        let client = client(&listen(fake.clone()).await).await;
        let (status, body, _) = post(&client, path).await;
        assert_eq!(status, Status::NotImplemented, "{path}: {body}");
        assert!(body.to_string().contains(ability), "{path}: {body}");
        assert!(
            fake.writes().is_empty(),
            "{path} wrote to a camera without {ability}"
        );
        assert!(
            fake.frames().iter().any(|frame| frame.id == msg::ABILITY),
            "{path} never read the abilities"
        );
        assert_eq!(fake.presets.lock().unwrap().len(), 1);
        assert_eq!(*fake.tour.lock().unwrap(), vec![1]);
    }
}

#[tokio::test]
async fn every_gated_write_goes_out_when_the_device_reports_its_ability() {
    for (_, path) in REFUSALS {
        let fake = device(Mood::Answers, &[(1, "Gate")], &[1]);
        let client = client(&listen(fake.clone()).await).await;
        let (status, body, _) = post(&client, path).await;
        assert_eq!(status, Status::Ok, "{path}: {body}");
        assert!(!fake.writes().is_empty(), "{path}");
        let _ = post(&client, "/api/device/controls/ptz/release?pressed_ms=900").await;
    }
}

#[tokio::test]
async fn pan_and_tilt_do_not_need_the_zoom_ability() {
    let fake = device(Mood::Answers, &[], &[]).without("SupportBallTelescopic");
    let client = client(&listen(fake.clone()).await).await;
    let (status, body, _) = post(&client, "/api/device/controls/ptz/press?action=left").await;
    assert_eq!(status, Status::Ok, "{body}");
    let (status, _, _) = post(&client, "/api/device/controls/ptz/release?pressed_ms=900").await;
    assert_eq!(status, Status::Ok);
    assert_eq!(fake.ptz().len(), 2);
}

#[tokio::test]
async fn a_hold_without_a_release_stops_itself_at_the_limit() {
    let fake = device(Mood::Answers, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    let (status, body, _) = post(&client, "/api/device/controls/ptz/press?action=right").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["limit_ms"], HOLD_MS);
    let deadline = Instant::now() + Duration::from_millis(HOLD_MS + 3000);
    while Instant::now() < deadline && !fake.ptz().iter().any(|frame| preset_of(frame) == -1) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let frames = fake.ptz();
    assert_eq!(frames.len(), 2, "the stop never went out");
    assert_eq!(command(&frames[1]), "DirectionRight");
    assert_eq!(preset_of(&frames[1]), -1);
    assert_eq!(frames[0].connection, frames[1].connection);
    let ran = frames[1].at - frames[0].at;
    assert!(
        ran >= Duration::from_millis(HOLD_MS - 100),
        "stopped after {ran:?}"
    );
    assert!(
        ran < Duration::from_millis(HOLD_MS + 1000),
        "stopped after {ran:?}"
    );
    let (status, body, _) = post(&client, "/api/device/controls/ptz/release?pressed_ms=4000").await;
    assert_eq!(status, Status::Ok);
    assert_eq!(body["held"], false);
}

#[tokio::test]
async fn a_release_that_beats_the_press_ack_stops_the_motor_right_after_the_press() {
    let fake = device(Mood::SlowAbility, &[], &[]);
    let client = client(&listen(fake.clone()).await).await;
    let (pressed, released) = tokio::join!(
        post(&client, "/api/device/controls/ptz/press?action=zoom-in"),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            post(&client, "/api/device/controls/ptz/release?pressed_ms=900").await
        }
    );
    assert_eq!(pressed.0, Status::Ok, "{}", pressed.1);
    assert_eq!(released.0, Status::Ok, "{}", released.1);
    assert_eq!(released.1["held"], true, "{}", released.1);
    let frames = fake.ptz();
    assert_eq!(frames.len(), 2, "press and stop");
    assert_eq!(command(&frames[0]), "ZoomTile");
    assert_eq!(preset_of(&frames[0]), 65535);
    assert_eq!(preset_of(&frames[1]), -1);
    let gap = frames[1].at - frames[0].at;
    assert!(
        gap < Duration::from_millis(800),
        "the stop came {gap:?} after the press"
    );
}

#[tokio::test]
async fn a_release_queued_behind_a_refused_press_sends_nothing() {
    let fake = device(Mood::SlowAbility, &[], &[]).without("SupportBallTelescopic");
    let client = client(&listen(fake.clone()).await).await;
    let (pressed, released) = tokio::join!(
        post(&client, "/api/device/controls/ptz/press?action=zoom-in"),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            post(&client, "/api/device/controls/ptz/release?pressed_ms=900").await
        }
    );
    assert_eq!(pressed.0, Status::NotImplemented, "{}", pressed.1);
    assert_eq!(released.0, Status::Ok, "{}", released.1);
    assert_eq!(released.1["held"], false, "{}", released.1);
    assert!(fake.writes().is_empty());
}
