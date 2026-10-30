use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use proview_core::dvrip::frame::Header;
use proview_core::dvrip::{msg, HEADER_LEN};
use proview_core::provider::Mode;
use proview_server::config::ModeStore;
use rocket::http::{ContentType, Status};
use rocket::local::asynchronous::Client;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const OUTER: Duration = Duration::from_secs(6);

#[derive(Default)]
struct Device {
    blocks: HashMap<String, Value>,
    abilities: Value,
    sets: Vec<(String, Value)>,
    silent_reads: bool,
    ignore_sets: bool,
}

type Shared = Arc<Mutex<Device>>;

fn camera() -> Device {
    let mut blocks = HashMap::new();
    blocks.insert(
        "Camera.Param".to_string(),
        json!([{ "DayNightColor": "0x00000003", "PictureFlip": "0x00000000" }]),
    );
    blocks.insert("Camera.ParamEx".to_string(), json!([{ "LowLuxMode": 1 }]));
    blocks.insert(
        "Camera.ClearFog".to_string(),
        json!([{ "enable": false, "level": 50 }]),
    );
    blocks.insert(
        "Camera.WhiteLight".to_string(),
        json!({
            "Brightness": 50,
            "MoveTrigLight": { "Duration": 60, "Level": 3 },
            "WorkMode": "Intelligent",
            "WorkPeriod": { "EHour": 6, "EMinute": 0, "Enable": 1, "SHour": 18, "SMinute": 0 }
        }),
    );
    blocks.insert("General.OneKeyMaskVideo".to_string(), json!([null]));
    blocks.insert("General.TimingSleep".to_string(), json!([null]));
    blocks.insert(
        "FbExtraStateCtrl".to_string(),
        json!({ "PlayVoiceTip": 1, "ison": 1 }),
    );
    blocks.insert(
        "fVideo.Volume".to_string(),
        json!([{ "AudioMode": "Single", "LeftVolume": 50, "RightVolume": 50 }]),
    );
    blocks.insert("fVideo.InVolumeIn".to_string(), json!([null]));
    blocks.insert(
        "fVideo.VolumeIn".to_string(),
        json!([{ "AnrOpen": true, "AudioMode": "Single", "LeftVolume": 34, "RightVolume": 35 }]),
    );
    Device {
        blocks,
        abilities: json!({ "OtherFunction": {
            "SupportCameraWhiteLight": true,
            "SupportCloseVoiceTip": true,
            "SupportSetVolume": true,
            "SupportSoftPhotosensitive": true
        }}),
        ..Device::default()
    }
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

async fn fake(device: Device) -> (String, Shared) {
    std::env::set_var("PROVIEW_SETTINGS_WINDOW_MS", "400");
    let shared = Arc::new(Mutex::new(device));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let state = shared.clone();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(serve(socket, state.clone()));
        }
    });
    (address, shared)
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

fn answer(state: &Shared, id: u16, body: &Value) -> Option<Value> {
    let mut device = state.lock().unwrap();
    let name = body
        .get("Name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    match id {
        msg::LOGIN => Some(json!({ "Ret": 100, "SessionID": "0x11223344" })),
        msg::ABILITY => Some(json!({ "Ret": 100, "SystemFunction": device.abilities.clone() })),
        msg::CONFIG_GET if device.silent_reads => None,
        msg::CONFIG_GET => Some(match device.blocks.get(&name) {
            Some(value) => json!({ "Ret": 100, "Name": name.clone(), name: value.clone() }),
            None => json!({ "Ret": 607, "Name": name.clone(), name: Value::Null }),
        }),
        msg::CONFIG_SET => {
            let value = body.get(&name).cloned().unwrap_or(Value::Null);
            device.sets.push((name.clone(), value.clone()));
            if !device.ignore_sets {
                device.blocks.insert(name, value);
            }
            Some(json!({ "Ret": 100 }))
        }
        _ => Some(json!({ "Ret": 100 })),
    }
}

async fn serve(mut socket: TcpStream, state: Shared) {
    loop {
        let mut header_bytes = [0u8; HEADER_LEN];
        if socket.read_exact(&mut header_bytes).await.is_err() {
            return;
        }
        let Ok(header) = Header::decode(&header_bytes) else {
            return;
        };
        let mut body = vec![0u8; header.length as usize];
        if header.length > 0 && socket.read_exact(&mut body).await.is_err() {
            return;
        }
        let parsed = serde_json::from_slice::<Value>(trim(&body)).unwrap_or(Value::Null);
        let Some(value) = answer(&state, header.id, &parsed) else {
            std::future::pending::<()>().await;
            return;
        };
        if socket.write_all(&reply(&header, &value)).await.is_err() {
            return;
        }
    }
}

async fn client(device: &str) -> Client {
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

async fn call(client: &Client, post: bool, path: &str) -> (Status, Value) {
    let request = if post {
        client.post(path)
    } else {
        client.get(path)
    };
    let response = tokio::time::timeout(OUTER, request.dispatch())
        .await
        .unwrap_or_else(|_| panic!("{path} never answered inside {OUTER:?}"));
    let status = response.status();
    let text = response.into_string().await.unwrap_or_default();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

#[tokio::test]
async fn the_report_reads_every_block_and_gates_by_ability() {
    let (address, _state) = fake(camera()).await;
    let client = client(&address).await;
    let (status, body) = call(&client, false, "/api/device/image").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["day_night"]["value"]["code"], 3);
    assert_eq!(
        body["defog"]["value"],
        json!({ "enable": false, "level": 50 })
    );
    assert_eq!(body["white_light"]["value"]["period"]["start"], "18:00");
    assert_eq!(body["voice_tip"]["supported"], true);
    assert_eq!(body["status_led"]["supported"], false);
    assert_eq!(body["low_lux"]["supported"], false);
    assert_eq!(body["privacy"]["supported"], false);
    assert_eq!(body["sleep"]["supported"], false);
    assert_eq!(
        body["mic_volume"]["value"],
        json!({ "left": 34, "right": 35, "block": "fVideo.VolumeIn" })
    );
}

#[tokio::test]
async fn a_mic_write_lands_on_the_block_the_firmware_fills_and_reads_back() {
    let (address, state) = fake(camera()).await;
    let client = client(&address).await;
    let (status, body) = call(
        &client,
        true,
        "/api/device/image/volume?target=mic&left=60&right=61",
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["block"], "fVideo.VolumeIn");
    assert_eq!(body["matches"], true);
    let device = state.lock().unwrap();
    assert_eq!(device.sets.len(), 1);
    assert_eq!(device.sets[0].0, "fVideo.VolumeIn");
    assert_eq!(device.sets[0].1[0]["LeftVolume"], 60);
    assert_eq!(device.sets[0].1[0]["RightVolume"], 61);
    assert_eq!(device.sets[0].1[0]["AnrOpen"], true);
}

#[tokio::test]
async fn a_control_without_its_ability_flag_is_refused_before_any_write() {
    let (address, state) = fake(camera()).await;
    let client = client(&address).await;
    for path in [
        "/api/device/image/indicators?led=false",
        "/api/device/image/privacy?enable=true",
        "/api/device/image/sleep?enable=true&start=03:00&end=03:10",
        "/api/device/image/low-lux?enable=false",
    ] {
        let (status, body) = call(&client, true, path).await;
        assert_eq!(status, Status::NotImplemented, "{path}: {body}");
        assert!(
            body["message"]
                .as_str()
                .unwrap_or("")
                .contains("SystemFunction.OtherFunction"),
            "{path}: {body}"
        );
    }
    assert!(state.lock().unwrap().sets.is_empty());
}

#[tokio::test]
async fn privacy_and_sleep_write_on_a_device_that_declares_them() {
    let mut device = camera();
    device.abilities["OtherFunction"]["SupportOneKeyMaskVideo"] = json!(true);
    device.abilities["OtherFunction"]["SupportTimingSleep"] = json!(true);
    device.blocks.insert(
        "General.OneKeyMaskVideo".to_string(),
        json!([{ "Enable": false }]),
    );
    device.blocks.insert(
        "General.TimingSleep".to_string(),
        json!([{ "Enable": false, "InSleeping": false, "ManualWakeUp": false, "RepeatType": 0,
                  "WorkPeriod": { "SHour": 0, "SMinute": 0, "EHour": 1, "EMinute": 0 } }]),
    );
    let (address, state) = fake(device).await;
    let client = client(&address).await;
    let (status, body) = call(&client, true, "/api/device/image/privacy?enable=true").await;
    assert_eq!(status, Status::Ok, "{body}");
    let (status, body) = call(
        &client,
        true,
        "/api/device/image/sleep?enable=true&start=03:10&end=03:20&repeat=false",
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let (_, report) = call(&client, false, "/api/device/image").await;
    assert_eq!(report["privacy"]["value"], true);
    let (status, live) = call(&client, false, "/api/device/image/live").await;
    assert_eq!(status, Status::Ok, "{live}");
    assert_eq!(live["privacy"]["value"], true);
    assert_eq!(live["sleep"]["value"]["enable"], true);
    assert_eq!(live["low_lux"]["supported"], false);
    assert_eq!(report["sleep"]["value"]["start"], "03:10");
    assert_eq!(report["sleep"]["value"]["enable"], true);
    assert_eq!(state.lock().unwrap().sets.len(), 2);
}

#[tokio::test]
async fn a_write_the_device_does_not_keep_answers_conflict() {
    let (address, state) = fake(camera()).await;
    let client = client(&address).await;
    let (status, body) = call(
        &client,
        true,
        "/api/device/image/defog?enable=true&level=70",
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["after"][0]["level"], 70);
    state.lock().unwrap().ignore_sets = true;
    let (status, body) = call(&client, true, "/api/device/image/day-night?code=1").await;
    assert_eq!(status, Status::Conflict, "{body}");
    assert_eq!(body["ok"], false);
    assert_eq!(body["requested"][0]["DayNightColor"], "0x00000001");
    assert_eq!(body["after"][0]["DayNightColor"], "0x00000003");
}

#[tokio::test]
async fn invalid_values_are_refused_before_the_device_is_touched() {
    let (address, state) = fake(camera()).await;
    let client = client(&address).await;
    for path in [
        "/api/device/image/light?start=06:00",
        "/api/device/image/light?level=2",
        "/api/device/image/day-night?code=9",
        "/api/device/image/volume?target=speaker&level=101",
        "/api/device/image/volume?target=horn&level=10",
    ] {
        let (status, body) = call(&client, true, path).await;
        assert_eq!(status, Status::BadRequest, "{path}: {body}");
    }
    assert!(state.lock().unwrap().sets.is_empty());
}

#[tokio::test]
async fn a_device_silent_on_config_get_answers_504_naming_the_stage() {
    let mut device = camera();
    device.silent_reads = true;
    let (address, _state) = fake(device).await;
    let client = client(&address).await;
    for (post, path, stage) in [
        (false, "/api/device/image", "Camera.Param read"),
        (
            true,
            "/api/device/image/defog?enable=true",
            "Camera.ClearFog read",
        ),
        (
            true,
            "/api/device/image/volume?target=speaker&level=40",
            "fVideo.Volume read",
        ),
    ] {
        let (status, body) = call(&client, post, path).await;
        assert_eq!(status, Status::GatewayTimeout, "{path}: {body}");
        let message = body["message"].as_str().unwrap_or("");
        assert!(message.contains(stage), "{path}: {body}");
        assert!(
            message.contains("did not answer inside the image settings window"),
            "{path}: {body}"
        );
    }
}
