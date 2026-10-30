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

const OUTER: Duration = Duration::from_secs(4);

#[derive(Default)]
struct Device {
    abilities: Value,
    configs: HashMap<String, Value>,
    sent: Vec<(u16, Value)>,
    silent_config_get: bool,
}

type Shared = Arc<Mutex<Device>>;

fn motion() -> Value {
    json!({
        "AlarmType": 0,
        "Enable": true,
        "Level": 3,
        "Region": vec!["0xFFFFFFFF"; 32],
        "EventHandler": {
            "MessageEnable": true,
            "RecordEnable": true,
            "SnapEnable": true,
            "VoiceEnable": false,
            "VoiceType": 24,
            "TimeSection": vec![vec!["1 00:00:00-24:00:00", "0 00:00:00-24:00:00"]; 7],
        }
    })
}

fn device(manual_stop: bool) -> Shared {
    let mut configs = HashMap::new();
    configs.insert("Detect.MotionDetect.[0]".to_string(), motion());
    configs.insert(
        "Ability.VoiceTipType".to_string(),
        json!({ "VoiceTip": [{ "VoiceEnum": 24, "VoiceText": "Bell Alarm" }, { "VoiceEnum": 523, "VoiceText": "Entered" }] }),
    );
    configs.insert(
        "NetWork.PMS".to_string(),
        json!({ "Enable": true, "PushInterval": 10, "ServName": "push.example.invalid", "Port": 80, "BoxID": "" }),
    );
    Arc::new(Mutex::new(Device {
        abilities: json!({
            "AlarmFunction": {
                "MotionDetect": true,
                "PEAInHumanPed": false,
                "ManuIntellAlertAlarm": manual_stop,
            },
            "OtherFunction": { "SupportAlarmVoiceTips": true }
        }),
        configs,
        ..Default::default()
    }))
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

fn env() {
    std::env::set_var("PROVIEW_SETTINGS_WINDOW_MS", "400");
    std::env::set_var(
        "PROVIEW_SETTINGS_BACKUP_DIR",
        std::env::temp_dir().join(format!("pvw-detect-alarm-{}", std::process::id())),
    );
}

async fn fake(shared: Shared) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(serve(socket, shared.clone()));
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

fn answer(shared: &Shared, id: u16, body: &Value) -> Option<Value> {
    let mut device = shared.lock().unwrap();
    device.sent.push((id, body.clone()));
    let name = body
        .get("Name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    match id {
        msg::LOGIN => Some(json!({ "Ret": 100, "SessionID": "0x11223344" })),
        msg::ABILITY => Some(match name.as_str() {
            "SystemFunction" => json!({ "Ret": 100, "SystemFunction": device.abilities.clone() }),
            _ => json!({ "Ret": 103 }),
        }),
        msg::CONFIG_GET if device.silent_config_get => None,
        msg::CONFIG_GET => Some(match device.configs.get(&name) {
            Some(value) => {
                let mut out = json!({ "Ret": 100, "Name": name });
                out[&name] = value.clone();
                out
            }
            None => json!({ "Ret": 607, "Name": name }),
        }),
        msg::CONFIG_SET => {
            let value = body.get(&name).cloned().unwrap_or(Value::Null);
            device.configs.insert(name, value);
            Some(json!({ "Ret": 100 }))
        }
        _ => Some(json!({ "Ret": 100 })),
    }
}

async fn serve(mut socket: TcpStream, shared: Shared) {
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
        let Some(value) = answer(&shared, header.id, &parsed) else {
            std::future::pending::<()>().await;
            return;
        };
        if socket.write_all(&reply(&header, &value)).await.is_err() {
            return;
        }
    }
}

async fn client(shared: Shared) -> Client {
    env();
    let address = fake(shared).await;
    let client = Client::tracked(proview_server::build_with_modes(
        figment(),
        config(&address),
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

async fn get(client: &Client, path: &str) -> (Status, Value) {
    let response = tokio::time::timeout(OUTER, client.get(path).dispatch())
        .await
        .unwrap_or_else(|_| panic!("{path} never answered inside {OUTER:?}"));
    let status = response.status();
    let body = response.into_string().await.unwrap_or_default();
    (
        status,
        serde_json::from_str(&body).unwrap_or(Value::String(body)),
    )
}

async fn post(client: &Client, path: &str, body: Value) -> (Status, Value) {
    let response = tokio::time::timeout(
        OUTER,
        client
            .post(path)
            .header(ContentType::JSON)
            .body(body.to_string())
            .dispatch(),
    )
    .await
    .unwrap_or_else(|_| panic!("{path} never answered inside {OUTER:?}"));
    let status = response.status();
    let body = response.into_string().await.unwrap_or_default();
    (
        status,
        serde_json::from_str(&body).unwrap_or(Value::String(body)),
    )
}

fn sets(shared: &Shared) -> Vec<Value> {
    shared
        .lock()
        .unwrap()
        .sent
        .iter()
        .filter(|(id, _)| *id == msg::CONFIG_SET)
        .map(|(_, body)| body.clone())
        .collect()
}

fn detector<'a>(overview: &'a Value, id: &str) -> &'a Value {
    overview["overview"]["detectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .unwrap()
}

#[tokio::test]
async fn the_overview_gates_every_block_on_the_ability_flags() {
    let shared = device(false);
    let client = client(shared.clone()).await;
    let (status, body) = get(&client, "/api/device/detect").await;
    assert_eq!(status, Status::Ok, "{body}");
    let motion = detector(&body, "motion");
    assert_eq!(motion["gate"]["supported"], json!(true));
    assert_eq!(motion["state"]["level"], json!(3));
    assert!(motion["state"].get("region").is_none(), "{motion}");
    let human = detector(&body, "human");
    assert_eq!(human["gate"]["supported"], json!(false));
    assert!(human["gate"]["evidence"]
        .as_str()
        .unwrap()
        .contains("PEAInHumanPed=false"));
    assert_eq!(detector(&body, "car")["state"], Value::Null);
    assert!(body["overview"].get("grid").is_none());
    let ids: Vec<&str> = body["overview"]["detectors"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert!(!ids.contains(&"loss") && !ids.contains(&"blind"), "{ids:?}");
    assert_eq!(body["overview"]["tones"][1]["id"], json!(523));
    assert_eq!(body["overview"]["manual_stop"]["supported"], json!(false));
    let names: Vec<String> = shared
        .lock()
        .unwrap()
        .sent
        .iter()
        .filter(|(id, _)| *id == msg::CONFIG_GET)
        .filter_map(|(_, body)| body["Name"].as_str().map(str::to_string))
        .collect();
    assert!(!names
        .iter()
        .any(|name| name.contains("HumanDetection") || name.contains("CarShape")));
}

#[tokio::test]
async fn a_motion_write_sends_the_channel_block_and_reads_it_back() {
    let shared = device(false);
    let client = client(shared.clone()).await;
    let (status, body) = post(
        &client,
        "/api/device/detect/motion",
        json!({ "level": 6, "push": false }),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["report"]["reverted"], json!(false));
    let fields: Vec<&str> = body["report"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["field"].as_str().unwrap())
        .collect();
    assert_eq!(fields, vec!["Level", "EventHandler.MessageEnable"]);
    let sent = sets(&shared);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["Name"], json!("Detect.MotionDetect.[0]"));
    let block = &sent[0]["Detect.MotionDetect.[0]"];
    assert_eq!(block["Level"], json!(6));
    assert_eq!(block["EventHandler"]["MessageEnable"], json!(false));
    assert_eq!(block["EventHandler"]["VoiceType"], json!(24));
    assert_eq!(block["Region"], motion()["Region"]);
}

#[tokio::test]
async fn a_tone_outside_the_list_and_a_bad_level_never_reach_the_device() {
    let shared = device(false);
    let client = client(shared.clone()).await;
    let (status, body) = post(
        &client,
        "/api/device/detect/motion",
        json!({ "voice_type": 999 }),
    )
    .await;
    assert_eq!(status, Status::BadRequest, "{body}");
    let (status, body) = post(&client, "/api/device/detect/motion", json!({ "level": 9 })).await;
    assert_eq!(status, Status::BadRequest, "{body}");
    let (status, body) = post(&client, "/api/device/detect/motion", json!({ "levle": 2 })).await;
    assert_eq!(status, Status::BadRequest, "{body}");
    let (status, _) = post(
        &client,
        "/api/device/detect/siren",
        json!({ "enable": true }),
    )
    .await;
    assert_eq!(status, Status::NotFound);
    assert!(sets(&shared).is_empty());
}

#[tokio::test]
async fn the_push_block_takes_only_an_interval_inside_the_app_range() {
    let shared = device(false);
    let client = client(shared.clone()).await;
    for patch in [
        json!({ "enable": false }),
        json!({ "push_interval": 29 }),
        json!({ "push_interval": 1801 }),
        json!({ "enable": true, "push_interval": 60 }),
        json!({ "push": false }),
    ] {
        let (status, body) = post(&client, "/api/device/detect/push", patch.clone()).await;
        assert_eq!(status, Status::BadRequest, "{patch}: {body}");
    }
    let (status, body) = post(
        &client,
        "/api/device/detect/motion",
        json!({ "region": [[true]] }),
    )
    .await;
    assert_eq!(status, Status::BadRequest, "{body}");
    for gone in ["loss", "blind"] {
        let (status, _) = post(
            &client,
            &format!("/api/device/detect/{gone}"),
            json!({ "enable": true }),
        )
        .await;
        assert_eq!(status, Status::NotFound, "{gone}");
    }
    assert!(sets(&shared).is_empty());
    let (status, body) = post(
        &client,
        "/api/device/detect/push",
        json!({ "push_interval": 30 }),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let sent = sets(&shared);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["NetWork.PMS"]["PushInterval"], json!(30));
    assert_eq!(sent[0]["NetWork.PMS"]["Enable"], json!(true));
}

#[tokio::test]
async fn an_unadvertised_detector_answers_501_with_the_flag_and_writes_nothing() {
    let shared = device(false);
    let client = client(shared.clone()).await;
    for key in [
        "car",
        "pet",
        "cry",
        "volume",
        "intell",
        "remote_call",
        "human",
    ] {
        let (status, body) = post(
            &client,
            &format!("/api/device/detect/{key}"),
            json!({ "enable": true }),
        )
        .await;
        assert_eq!(status, Status::NotImplemented, "{key}: {body}");
        assert!(
            body.to_string().contains("not offered by this camera"),
            "{key}: {body}"
        );
    }
    assert!(sets(&shared).is_empty());
}

#[tokio::test]
async fn stop_alarm_sends_only_the_stop_frame_when_the_app_would_show_the_button() {
    let shared = device(true);
    let client = client(shared.clone()).await;
    let (status, body) = get(&client, "/api/device/detect/alarm/stop").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["gate"]["supported"], json!(true));
    let (status, body) = post(&client, "/api/device/detect/alarm/stop", json!({})).await;
    assert_eq!(status, Status::Ok, "{body}");
    let frames: Vec<Value> = shared
        .lock()
        .unwrap()
        .sent
        .iter()
        .filter(|(id, _)| *id == msg::OP_REMOTE_CTRL)
        .map(|(_, body)| body.clone())
        .collect();
    assert_eq!(frames.len(), 1);
    assert_eq!(
        frames[0]["OPRemoteCtrl"],
        json!({ "Type": "ManuIntelAlarm", "msg": "0x00000000", "P1": "0x0", "P2": "0x0" })
    );
    assert_eq!(frames[0]["Name"], json!("OPRemoteCtrl"));
}

#[tokio::test]
async fn stop_alarm_is_refused_without_the_manual_alarm_flag() {
    let shared = device(false);
    let client = client(shared.clone()).await;
    let (status, body) = get(&client, "/api/device/detect/alarm/stop").await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["gate"]["supported"], json!(false));
    assert!(body["gate"]["evidence"]
        .as_str()
        .unwrap()
        .contains("ManuIntellAlertAlarm"));
    let (status, body) = post(&client, "/api/device/detect/alarm/stop", json!({})).await;
    assert_eq!(status, Status::NotImplemented, "{body}");
    assert!(body.to_string().contains("ManuIntellAlertAlarm"), "{body}");
    assert!(!shared
        .lock()
        .unwrap()
        .sent
        .iter()
        .any(|(id, _)| *id == msg::OP_REMOTE_CTRL));
}

#[tokio::test]
async fn a_device_silent_on_config_reads_times_out_with_the_stage_named() {
    let shared = device(false);
    shared.lock().unwrap().silent_config_get = true;
    let client = client(shared.clone()).await;
    let bound = Duration::from_millis(400 + 250);
    let started = std::time::Instant::now();
    let (status, body) = get(&client, "/api/device/detect").await;
    let read = started.elapsed();
    assert_eq!(status, Status::GatewayTimeout, "{body}");
    assert!(body.to_string().contains("Ability.VoiceTipType"), "{body}");
    assert!(read < bound, "the read answered after {read:?}");
    let started = std::time::Instant::now();
    let (status, body) = post(&client, "/api/device/detect/motion", json!({ "level": 6 })).await;
    let write = started.elapsed();
    assert_eq!(status, Status::GatewayTimeout, "{body}");
    assert!(
        body.to_string().contains("Detect.MotionDetect.[0]"),
        "{body}"
    );
    assert!(write < bound, "the write answered after {write:?}");
    assert!(sets(&shared).is_empty());
}
