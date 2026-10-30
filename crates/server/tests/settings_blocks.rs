use std::collections::HashMap;
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

const WINDOW_MS: &str = "400";
const OUTER: Duration = Duration::from_secs(5);
const SESSION: &str = "0x11223344";

#[derive(Default)]
struct Device {
    blocks: HashMap<String, Value>,
    titles: Vec<String>,
    frames: Vec<(u16, Vec<u8>)>,
    silent: Vec<u16>,
}

type Shared = Arc<Mutex<Device>>;

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

fn settings_env() {
    std::env::set_var("PROVIEW_SETTINGS_WINDOW_MS", WINDOW_MS);
    std::env::set_var("PROVIEW_FORMAT_WINDOW_MS", WINDOW_MS);
    std::env::set_var(
        "PROVIEW_SETTINGS_BACKUP_DIR",
        std::env::temp_dir().join(format!("pvw-settings-blocks-{}", std::process::id())),
    );
}

fn camera() -> Device {
    let mut blocks = HashMap::new();
    blocks.insert(
        "StorageInfo".to_string(),
        json!([
            {
                "PartNumber": 3,
                "Partition": [
                    { "DirverType": 0, "IsCurrent": true, "Status": 0, "TotalSpace": "0x0000e42b", "RemainSpace": "0x00000400" },
                    { "DirverType": 4, "IsCurrent": true, "Status": 0, "TotalSpace": "0x00000405", "RemainSpace": "0x00000000" },
                    { "DirverType": 0, "IsCurrent": false, "Status": 0, "TotalSpace": "0x00000000", "RemainSpace": "0x00000000" }
                ]
            }
        ]),
    );
    blocks.insert(
        "General.General".to_string(),
        json!({ "OverWrite": "OverWrite", "AutoLogout": 0 }),
    );
    blocks.insert(
        "Storage.StorageNotExist".to_string(),
        json!({ "Enable": false, "EventHandler": {} }),
    );
    let days = |first: &str, rest: &str| -> Value {
        json!((0..7)
            .map(|_| {
                let mut row = vec![first.to_string()];
                row.extend((0..5).map(|_| rest.to_string()));
                row
            })
            .collect::<Vec<_>>())
    };
    blocks.insert(
        "Record".to_string(),
        json!([{
            "Mask": days("0x7", "0x00000000"),
            "PacketLength": 5,
            "PreRecord": 5,
            "RecordMode": "ConfigRecord",
            "Redundancy": false,
            "TimeSection": days("1 00:00:00-24:00:00", "0 00:00:00-24:00:00"),
        }]),
    );
    blocks.insert(
        "Simplify.Encode".to_string(),
        json!([{
            "MainFormat": { "AudioEnable": true, "Video": { "BitRate": 1536, "BitRateControl": "VBR", "Compression": "H.265", "FPS": 12, "GOP": 3, "Quality": 3, "Resolution": "6M" }, "VideoEnable": true },
            "ExtraFormat": { "AudioEnable": true, "Video": { "BitRate": 1024, "BitRateControl": "VBR", "Compression": "H.265", "FPS": 12, "GOP": 3, "Quality": 3, "Resolution": "HD1" }, "VideoEnable": true }
        }]),
    );
    blocks.insert(
        "EncodeCapability".to_string(),
        json!({
            "MaxBitrate": 131072,
            "MaxEncodePowerPerChannel": ["0x07800000"],
            "ImageSizePerChannel": ["0x00204000"],
            "ExImageSizePerChannelEx": [(0..32).map(|_| "0x0000000A").collect::<Vec<_>>()],
            "EncodeInfo": [{ "ResolutionMask": "0x00204000" }],
            "CombEncodeInfo": [{ "ResolutionMask": "0x0000000A" }]
        }),
    );
    blocks.insert(
        "AVEnc.SmartH264V2".to_string(),
        json!([{ "Smart264PlusV2": [{ "SmartH264Plus": 0 }], "Smart264V2": [{ "SmartH264": false }] }]),
    );
    blocks.insert(
        "Encode264ability".to_string(),
        json!({ "uIntel264": ["0x00000000"], "uIntel264Plus": ["0x00000000"] }),
    );
    blocks.insert(
        "AVEnc.VideoWidget".to_string(),
        json!([{
            "ChannelTitle": { "Name": "serial-one door", "SerialNo": "1" },
            "ChannelTitleAttribute": { "EncodeBlend": true, "PreviewBlend": true },
            "TimeTitleAttribute": { "EncodeBlend": true, "PreviewBlend": true }
        }]),
    );
    Device {
        blocks,
        titles: vec!["serial-one door".to_string()],
        ..Device::default()
    }
}

async fn fake(device: Device) -> (String, Shared) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let shared = Arc::new(Mutex::new(device));
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

fn answer(device: &mut Device, id: u16, body: &[u8]) -> Option<Value> {
    if device.silent.contains(&id) {
        return None;
    }
    let parsed = serde_json::from_slice::<Value>(trim(body)).unwrap_or(Value::Null);
    let name = parsed
        .get("Name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(match id {
        msg::LOGIN => json!({ "Ret": 100, "SessionID": SESSION }),
        msg::CONFIG_GET | msg::SYSTEM_INFO | msg::ABILITY => match device.blocks.get(&name) {
            Some(value) => json!({ "Ret": 100, "Name": name, name.clone(): value }),
            None => json!({ "Ret": 607, "Name": name }),
        },
        msg::CONFIG_SET => {
            device.blocks.insert(name.clone(), parsed[&name].clone());
            json!({ "Ret": 100, "Name": name })
        }
        msg::CHANNEL_TITLE_GET => {
            json!({ "Ret": 100, "Name": "ChannelTitle", "ChannelTitle": device.titles })
        }
        msg::CHANNEL_TITLE_SET => {
            device.titles = parsed["ChannelTitle"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|item| item.as_str().unwrap_or_default().to_string())
                        .collect()
                })
                .unwrap_or_default();
            json!({ "Ret": 100, "Name": "ChannelTitle" })
        }
        _ => json!({ "Ret": 100 }),
    })
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
        let value = {
            let mut device = shared.lock().unwrap();
            device.frames.push((header.id, body.clone()));
            answer(&mut device, header.id, &body)
        };
        let Some(value) = value else {
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

async fn call(client: &Client, method: &str, path: &str, body: Option<Value>) -> (Status, Value) {
    let request = match method {
        "GET" => client.get(path),
        "PUT" => client.put(path),
        _ => client.post(path),
    };
    let request = match body {
        Some(body) => request.header(ContentType::JSON).body(body.to_string()),
        None => request,
    };
    let response = tokio::time::timeout(OUTER, request.dispatch())
        .await
        .unwrap_or_else(|_| panic!("{method} {path} never answered inside {OUTER:?}"));
    let status = response.status();
    let text = response.into_string().await.unwrap_or_default();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

fn sent(shared: &Shared, id: u16) -> Vec<Vec<u8>> {
    shared
        .lock()
        .unwrap()
        .frames
        .iter()
        .filter(|(frame, _)| *frame == id)
        .map(|(_, body)| body.clone())
        .collect()
}

fn sent_json(shared: &Shared, id: u16) -> Vec<Value> {
    sent(shared, id)
        .iter()
        .map(|body| serde_json::from_slice(trim(body)).unwrap())
        .collect()
}

#[tokio::test]
async fn format_without_the_typed_confirmation_sends_nothing_to_the_card() {
    settings_env();
    let (address, shared) = fake(camera()).await;
    let client = client(&address).await;
    for path in [
        "/api/device/settings/storage/format",
        "/api/device/settings/storage/format?confirm=format",
        "/api/device/settings/storage/format?confirm=yes",
    ] {
        let (status, body) = call(&client, "POST", path, None).await;
        assert_eq!(status, Status::BadRequest, "{path}: {body}");
        assert!(
            body["message"].as_str().unwrap().contains("confirm=FORMAT"),
            "{body}"
        );
    }
    assert!(sent(&shared, msg::STORAGE_MANAGER).is_empty());
}

#[tokio::test]
async fn format_clears_every_partition_with_space_using_the_app_wire_json() {
    settings_env();
    let (address, shared) = fake(camera()).await;
    let client = client(&address).await;
    let (status, body) = call(
        &client,
        "POST",
        "/api/device/settings/storage/format?confirm=FORMAT",
        None,
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["steps"].as_array().unwrap().len(), 2, "{body}");
    let frames = sent_json(&shared, msg::STORAGE_MANAGER);
    let expected = |part: u64| {
        json!({
            "Name": "OPStorageManager",
            "OPStorageManager": { "Action": "Clear", "PartNo": part, "SerialNo": 0, "Type": "Data" },
            "SessionID": SESSION,
        })
    };
    assert_eq!(frames, vec![expected(0), expected(1)]);
}

#[tokio::test]
async fn a_format_the_card_never_acknowledges_answers_504_inside_the_window() {
    settings_env();
    let mut device = camera();
    device.silent.push(msg::STORAGE_MANAGER);
    let (address, shared) = fake(device).await;
    let client = client(&address).await;
    let started = Instant::now();
    let (status, body) = call(
        &client,
        "POST",
        "/api/device/settings/storage/format?confirm=FORMAT",
        None,
    )
    .await;
    assert_eq!(status, Status::GatewayTimeout, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("OPStorageManager"),
        "{body}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(400 * 3 + 800),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(sent(&shared, msg::STORAGE_MANAGER).len(), 1);
}

#[tokio::test]
async fn storage_reports_card_usage_by_partition_kind_and_the_missing_card_alarm() {
    settings_env();
    let (address, _) = fake(camera()).await;
    let client = client(&address).await;
    let (status, body) = call(&client, "GET", "/api/device/settings/storage", None).await;
    assert_eq!(status, Status::Ok, "{body}");
    let usage = &body["storage"]["usage"];
    assert_eq!(usage["card"], "ok");
    assert_eq!(usage["record_mb"], 58411);
    assert_eq!(usage["snapshot_mb"], 1029);
    assert_eq!(usage["total_mb"], 59440);
    assert_eq!(usage["remain_mb"], 1024);
    assert_eq!(body["storage"]["missing_alarm"], false);
    assert_eq!(body["storage"]["partitions"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn the_missing_card_alarm_writes_enable_and_reads_it_back() {
    settings_env();
    let (address, shared) = fake(camera()).await;
    let client = client(&address).await;
    let (status, body) = call(
        &client,
        "PUT",
        "/api/device/settings/storage?missing_alarm=true",
        None,
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["storage"]["missing_alarm"], true);
    let writes = sent_json(&shared, msg::CONFIG_SET);
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0]["Storage.StorageNotExist"]["Enable"], true);
}

#[tokio::test]
async fn alarm_only_recording_writes_mask_six_in_the_device_format_for_every_day() {
    settings_env();
    let (address, shared) = fake(camera()).await;
    let client = client(&address).await;
    let (status, body) = call(
        &client,
        "PUT",
        "/api/device/settings/record?full_time=false&packet_length=10",
        None,
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["record"]["full_time"], false);
    assert_eq!(body["record"]["packet_length"], 10);
    let writes = sent_json(&shared, msg::CONFIG_SET);
    let record = &writes[0]["Record"][0];
    for day in 0..7 {
        assert_eq!(record["Mask"][day][0], "0x6");
        assert_eq!(record["Mask"][day][1], "0x00000000");
        assert_eq!(record["TimeSection"][day][0], "1 00:00:00-24:00:00");
        assert_eq!(record["TimeSection"][day][1], "0 00:00:00-24:00:00");
    }
    assert_eq!(record["RecordMode"], "ConfigRecord");
    assert_eq!(record["PacketLength"], 10);
}

#[tokio::test]
async fn a_clip_length_outside_the_app_range_is_refused_before_any_write() {
    settings_env();
    let (address, shared) = fake(camera()).await;
    let client = client(&address).await;
    for length in [4, 121] {
        let path = format!("/api/device/settings/record?packet_length={length}");
        let (status, body) = call(&client, "PUT", &path, None).await;
        assert_eq!(status, Status::BadRequest, "{body}");
    }
    assert!(sent(&shared, msg::CONFIG_SET).is_empty());
}

#[tokio::test]
async fn the_record_stream_is_refused_when_the_device_lacks_the_ability() {
    settings_env();
    let (address, shared) = fake(camera()).await;
    let client = client(&address).await;
    let (status, body) = call(
        &client,
        "PUT",
        "/api/device/settings/record?record_stream=extra",
        None,
    )
    .await;
    assert_eq!(status, Status::NotImplemented, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("SupportRecMainOrExtUseMainType"),
        "{body}"
    );
    assert!(sent(&shared, msg::CONFIG_SET).is_empty());
}

#[tokio::test]
async fn the_record_stream_writes_main_stream_one_for_the_extra_stream() {
    settings_env();
    let mut device = camera();
    device.blocks.insert(
        "SystemFunction".to_string(),
        json!({ "OtherFunction": { "SupportRecMainOrExtUseMainType": true } }),
    );
    device.blocks.insert(
        "Storage.RecordStream".to_string(),
        json!({ "ExtStream": 0, "MainStream": 0 }),
    );
    let (address, shared) = fake(device).await;
    let client = client(&address).await;
    let (status, body) = call(
        &client,
        "PUT",
        "/api/device/settings/record?record_stream=extra",
        None,
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["record"]["record_stream"], "extra");
    let writes = sent_json(&shared, msg::CONFIG_SET);
    assert_eq!(
        writes[0]["Storage.RecordStream"],
        json!({ "ExtStream": 0, "MainStream": 1 })
    );
}

#[tokio::test]
async fn smart_encoding_is_refused_when_encode264ability_reports_none() {
    settings_env();
    let (address, shared) = fake(camera()).await;
    let client = client(&address).await;
    let (status, body) = call(&client, "GET", "/api/device/settings/encode", None).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["smart"]["modes"], json!(["off"]));
    let (status, body) = call(
        &client,
        "PUT",
        "/api/device/settings/encode?smart=smart",
        None,
    )
    .await;
    assert_eq!(status, Status::BadRequest, "{body}");
    assert!(sent(&shared, msg::CONFIG_SET).is_empty());
}

#[tokio::test]
async fn smart_plus_writes_both_v2_flags_when_the_ability_allows_it() {
    settings_env();
    let mut device = camera();
    device.blocks.insert(
        "Encode264ability".to_string(),
        json!({ "uIntel264": ["0x00000001"], "uIntel264Plus": ["0x00000001"] }),
    );
    let (address, shared) = fake(device).await;
    let client = client(&address).await;
    let (status, body) = call(
        &client,
        "PUT",
        "/api/device/settings/encode?smart=smart_plus",
        None,
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["smart"]["mode"], "smart_plus");
    let writes = sent_json(&shared, msg::CONFIG_SET);
    assert_eq!(
        writes[0]["AVEnc.SmartH264V2"],
        json!([{ "Smart264PlusV2": [{ "SmartH264Plus": 1 }], "Smart264V2": [{ "SmartH264": true }] }])
    );
}

#[tokio::test]
async fn encode_fps_above_the_capability_ceiling_is_refused_before_any_write() {
    settings_env();
    let (address, shared) = fake(camera()).await;
    let client = client(&address).await;
    let (status, body) = call(&client, "GET", "/api/device/settings/encode", None).await;
    assert_eq!(status, Status::Ok, "{body}");
    let ceiling = body["capability"]["main"]["max_fps"]["6M"]
        .as_i64()
        .unwrap();
    assert!(ceiling < 25, "{body}");
    let path = format!(
        "/api/device/settings/encode?stream=main&fps={}",
        ceiling + 1
    );
    let (status, body) = call(&client, "PUT", &path, None).await;
    assert_eq!(status, Status::BadRequest, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("EncodeCapability"),
        "{body}"
    );
    let path = "/api/device/settings/encode?stream=main&resolution=4K";
    let (status, body) = call(&client, "PUT", path, None).await;
    assert_eq!(status, Status::BadRequest, "{body}");
    assert!(sent(&shared, msg::CONFIG_SET).is_empty());
}

#[tokio::test]
async fn a_rename_writes_the_widget_title_then_the_channel_title_list() {
    settings_env();
    let (address, shared) = fake(camera()).await;
    let client = client(&address).await;
    let (status, body) = call(
        &client,
        "PUT",
        "/api/device/settings/osd?name=Gate&time=false",
        None,
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["osd"]["name"], "Gate");
    assert_eq!(body["osd"]["titles"], json!(["Gate"]));
    assert_eq!(body["osd"]["time_shown"], false);
    let widget = sent_json(&shared, msg::CONFIG_SET);
    assert_eq!(
        widget[0]["AVEnc.VideoWidget"][0]["ChannelTitle"]["Name"],
        "Gate"
    );
    assert_eq!(
        widget[0]["AVEnc.VideoWidget"][0]["TimeTitleAttribute"]["EncodeBlend"],
        false
    );
    assert_eq!(
        widget[0]["AVEnc.VideoWidget"][0]["ChannelTitleAttribute"]["EncodeBlend"],
        true
    );
    let titles = sent_json(&shared, msg::CHANNEL_TITLE_SET);
    assert_eq!(
        titles,
        vec![json!({ "Name": "ChannelTitle", "ChannelTitle": ["Gate"], "SessionID": SESSION })]
    );
}

#[tokio::test]
async fn a_title_longer_than_the_app_limit_is_refused_before_any_write() {
    settings_env();
    let (address, shared) = fake(camera()).await;
    let client = client(&address).await;
    let path = format!("/api/device/settings/osd?name={}", "a".repeat(33));
    let (status, body) = call(&client, "PUT", &path, None).await;
    assert_eq!(status, Status::BadRequest, "{body}");
    assert!(sent(&shared, msg::CONFIG_SET).is_empty());
    assert!(sent(&shared, msg::CHANNEL_TITLE_SET).is_empty());
}

#[tokio::test]
async fn the_title_bitmap_goes_out_as_the_sdk_titledot_struct() {
    settings_env();
    let (address, shared) = fake(camera()).await;
    let client = client(&address).await;
    let bits = "ff00".repeat(24);
    let (status, body) = call(
        &client,
        "POST",
        "/api/device/settings/osd/dot",
        Some(json!({ "width": 16, "height": 24, "bits": bits })),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let frames = sent(&shared, msg::TITLE_DOT_SET);
    assert_eq!(frames.len(), 1);
    let frame = &frames[0];
    assert_eq!(&frame[..4], &[16, 0, 24, 0]);
    assert_eq!(&frame[4..16], &[0u8; 12]);
    assert_eq!(frame.len(), 16 + 48);
    assert_eq!(&frame[16..18], &[0xff, 0x00]);
    let (status, body) = call(
        &client,
        "POST",
        "/api/device/settings/osd/dot",
        Some(json!({ "width": 12, "height": 24, "bits": "00" })),
    )
    .await;
    assert_eq!(status, Status::BadRequest, "{body}");
    assert_eq!(sent(&shared, msg::TITLE_DOT_SET).len(), 1);
}

#[tokio::test]
async fn the_controls_settings_summary_reads_over_dvrip_without_onvif() {
    settings_env();
    let mut device = camera();
    device.blocks.insert(
        "SystemInfo".to_string(),
        json!({ "DeviceModel": "fake-model", "SerialNo": "serial-one" }),
    );
    device.blocks.insert(
        "NetWork.NetCommon".to_string(),
        json!({ "HostIP": "0x0A01A8C0", "Submask": "0x00FFFFFF" }),
    );
    let (address, _) = fake(device).await;
    let client = client(&address).await;
    let (status, body) = call(&client, "GET", "/api/device/controls/settings", None).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["backend"], "dvrip");
    assert_eq!(body["device"]["DeviceModel"], "fake-model");
    assert_eq!(body["encode"]["main"]["resolution"], "6M");
    assert_eq!(body["network"]["submask"], "0x00FFFFFF");
    assert_eq!(body["storage"]["usage"]["card"], "ok");
}
