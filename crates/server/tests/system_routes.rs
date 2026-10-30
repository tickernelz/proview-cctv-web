use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proview_core::dvrip::crypto::sofia_hash;
use proview_core::dvrip::frame::Header;
use proview_core::dvrip::{msg, HEADER_LEN};
use proview_core::provider::Mode;
use proview_server::config::ModeStore;
use rocket::http::{ContentType, Status};
use rocket::local::asynchronous::Client;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const WINDOW_MS: u64 = 400;
const OUTER: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mood {
    Talking,
    SilentOnSystemInfo,
}

#[derive(Default)]
struct Device {
    frames: Vec<(u16, Value, usize)>,
    time_min: i64,
    language: String,
    time: String,
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

fn env() {
    std::env::set_var("PROVIEW_SYSTEM_WINDOW_MS", WINDOW_MS.to_string());
    std::env::set_var(
        "PROVIEW_DATA_DIR",
        std::env::temp_dir().join(format!("pvw-system-routes-{}", std::process::id())),
    );
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

fn answer(device: &Shared, id: u16, body: &Value) -> Option<Value> {
    let name = body
        .get("Name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut state = device.lock().unwrap();
    Some(match id {
        msg::LOGIN => json!({ "Ret": 100, "SessionID": "0x11223344" }),
        msg::SYSTEM_INFO => match name.as_str() {
            "SystemInfo" => json!({ "Ret": 100, "SystemInfo": {
                "DeviceModel": "MODEL-ONE", "SerialNo": "serial-one",
                "SoftWareVersion": "V5.01.R02.ABCDEF01.10010.123456.0000000",
                "BuildTime": "2024-05-16 10:20:30", "HardWare": "HW-ONE",
                "DeviceRunTime": "0x0000003C", "Pid": "PID-ONE", "DeviceType": 24
            }}),
            "SystemInfoEx" => json!({ "Ret": 607 }),
            "XmCloudStatus" => json!({ "Ret": 100, "XmCloudStatus": { "Status": "Connected" } }),
            "WifiRouteInfo" => json!({ "Ret": 100, "WifiRouteInfo": { "SignalLevel": 70 } }),
            _ => json!({ "Ret": 607 }),
        },
        msg::ABILITY => match name.as_str() {
            "SystemFunction" => json!({ "Ret": 100, "SystemFunction": {
                "OtherFunction": { "SupportCfgCloudupgrade": true, "SupportGetMcuVersion": false },
                "NetServerFunction": { "WifiRouteSignalLevel": false }
            }}),
            "MultiLanguage" => json!({ "Ret": 100, "MultiLanguage": ["English", "Indonesian"] }),
            _ => json!({ "Ret": 607 }),
        },
        msg::CONFIG_GET => match name.as_str() {
            "System.TimeZone" => {
                json!({ "Ret": 100, "System.TimeZone": { "FirstUserTimeZone": 0, "timeMin": state.time_min } })
            }
            "General.Location" => {
                json!({ "Ret": 100, "General.Location": { "DSTRule": "Off", "DateFormat": "YYMMDD", "Language": state.language } })
            }
            "General.Location.Language" => {
                json!({ "Ret": 100, "General.Location.Language": state.language })
            }
            "NetWork.NetCommon" => {
                json!({ "Ret": 100, "NetWork.NetCommon": { "GateWay": "0x0101A8C0" } })
            }
            "Status.NatInfo" => {
                json!({ "Ret": 100, "Status.NatInfo": { "NatStatus": "Conneted" } })
            }
            _ => json!({ "Ret": 607 }),
        },
        msg::CONFIG_SET => {
            match name.as_str() {
                "System.TimeZone" => {
                    state.time_min = body["System.TimeZone"]["timeMin"].as_i64().unwrap_or(0)
                }
                "General.Location.Language" => {
                    state.language = body["General.Location.Language"]
                        .as_str()
                        .unwrap_or("")
                        .to_string()
                }
                _ => {}
            }
            json!({ "Ret": 100 })
        }
        msg::TIME_QUERY => json!({ "Ret": 100, "OPTimeQuery": state.time }),
        msg::OP_MACHINE => {
            if name == "OPTimeSetting" {
                state.time = body["OPTimeSetting"].as_str().unwrap_or("").to_string();
            }
            json!({ "Ret": 100 })
        }
        msg::PING => {
            json!({ "Ret": 100, "Ping": { "Result": true, "Avdelay": 9, "Lost": 0, "Num": 3, "URL": body["Ping"]["URL"] } })
        }
        msg::USERS_GET => json!({ "Ret": 100, "Users": [
            { "Name": "admin", "Password": "tlJwpbo6", "Group": "admin", "Memo": "admin 's account", "Reserved": true, "Sharable": true, "AuthorityList": ["ShutDown"] }
        ]}),
        msg::SAFETY => {
            let mut value = json!({ "Ret": 100 });
            value[name.as_str()] = json!({ "Question": 1, "VerifyQRCode": 0 });
            value
        }
        msg::VERSION_REQ => {
            json!({ "Ret": 100, "Name": "OPVersionReq", "OPVersionRep": { "OPVersionList": [] } })
        }
        msg::UPGRADE_DATA => return None,
        _ => json!({ "Ret": 100 }),
    })
}

async fn serve(mut socket: TcpStream, device: Shared, mood: Mood) {
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
        device
            .lock()
            .unwrap()
            .frames
            .push((header.id, parsed.clone(), body.len()));
        if mood == Mood::SilentOnSystemInfo && header.id == msg::SYSTEM_INFO {
            std::future::pending::<()>().await;
        }
        if header.id == msg::UPGRADE_DATA && header.length == 0 {
            let done = reply(&header, &json!({ "Ret": 100 }));
            if socket.write_all(&done).await.is_err() {
                return;
            }
            continue;
        }
        let Some(value) = answer(&device, header.id, &parsed) else {
            continue;
        };
        if socket.write_all(&reply(&header, &value)).await.is_err() {
            return;
        }
    }
}

async fn fake(mood: Mood) -> (String, Shared) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let device: Shared = Arc::new(Mutex::new(Device {
        time_min: -420,
        language: "English".to_string(),
        time: "2026-10-30 08:00:00".to_string(),
        ..Device::default()
    }));
    let shared = device.clone();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(serve(socket, shared.clone(), mood));
        }
    });
    (address, device)
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
        .unwrap_or_else(|_| panic!("{path} never answered inside {OUTER:?}"));
    let status = response.status();
    let text = response.into_string().await.unwrap_or_default();
    (
        status,
        serde_json::from_str(&text).unwrap_or(json!({ "raw": text })),
    )
}

fn frames_named(device: &Shared, id: u16, name: &str) -> Vec<Value> {
    device
        .lock()
        .unwrap()
        .frames
        .iter()
        .filter(|(frame_id, body, _)| {
            *frame_id == id && body.get("Name").and_then(Value::as_str) == Some(name)
        })
        .map(|(_, body, _)| body.clone())
        .collect()
}

fn ids(device: &Shared) -> Vec<u16> {
    device
        .lock()
        .unwrap()
        .frames
        .iter()
        .map(|(id, _, _)| *id)
        .collect()
}

#[tokio::test]
async fn about_reads_info_abilities_and_reports_missing_info_ex() {
    env();
    let (address, device) = fake(Mood::Talking).await;
    let client = client(&address).await;
    let (status, body) = call(&client, "GET", "/api/device/system", None).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["summary"]["model"], "MODEL-ONE");
    assert_eq!(body["summary"]["serial"], "serial-one");
    assert_eq!(body["summary"]["uptime_minutes"], 60);
    assert_eq!(body["gates"]["cloud_upgrade"], true);
    assert_eq!(body["gates"]["wifi_signal"], false);
    assert!(body["info_ex"].is_null());
    assert!(
        body["info_ex_error"].as_str().unwrap().contains("607"),
        "{body}"
    );
    assert_eq!(
        frames_named(&device, msg::SYSTEM_INFO, "SystemInfoEx").len(),
        1
    );
}

#[tokio::test]
async fn a_silent_device_times_the_about_page_out_inside_the_window() {
    env();
    let (address, _device) = fake(Mood::SilentOnSystemInfo).await;
    let client = client(&address).await;
    let started = Instant::now();
    let (status, body) = call(&client, "GET", "/api/device/system", None).await;
    let elapsed = started.elapsed();
    assert_eq!(status, Status::GatewayTimeout, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .starts_with("SystemInfo did not answer inside"),
        "{body}"
    );
    assert!(
        elapsed < Duration::from_millis(WINDOW_MS * 2 + 600),
        "{elapsed:?}"
    );
}

#[tokio::test]
async fn reboot_and_defaults_need_their_words_and_send_the_app_bodies() {
    env();
    let (address, device) = fake(Mood::Talking).await;
    let client = client(&address).await;
    let (status, _) = call(&client, "POST", "/api/device/system/reboot", None).await;
    assert_eq!(status, Status::BadRequest);
    let (status, _) = call(
        &client,
        "POST",
        "/api/device/system/defaults?confirm=reset",
        None,
    )
    .await;
    assert_eq!(status, Status::BadRequest);
    assert!(!ids(&device).contains(&msg::OP_MACHINE));
    let (status, body) = call(
        &client,
        "POST",
        "/api/device/system/reboot?confirm=reboot",
        None,
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let (status, body) = call(
        &client,
        "POST",
        "/api/device/system/defaults?confirm=RESET",
        None,
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let reboot = frames_named(&device, msg::OP_MACHINE, "OPMachine");
    assert_eq!(reboot.len(), 1);
    assert_eq!(reboot[0]["OPMachine"], json!({ "Action": "Reboot" }));
    let defaults = frames_named(&device, msg::OP_MACHINE, "OPDefaultConfig");
    assert_eq!(
        defaults[0]["OPDefaultConfig"],
        json!({ "Account": 1, "Alarm": 1, "CameraPARAM": 1, "CommPtz": 1, "Encode": 1, "General": 1, "NetCommon": 1, "NetServer": 1, "Preview": 1, "Record": 1 })
    );
}

#[tokio::test]
async fn time_sync_writes_zone_then_time_and_reads_both_back() {
    env();
    let (address, device) = fake(Mood::Talking).await;
    let client = client(&address).await;
    let (status, _) = call(
        &client,
        "POST",
        "/api/device/system/time?time=2026-10-30T10:00:00",
        None,
    )
    .await;
    assert_eq!(status, Status::BadRequest);
    let (status, body) = call(
        &client,
        "POST",
        "/api/device/system/time?time=2026-10-30%2010:11:12&offset=480",
        None,
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["device_time"], "2026-10-30 10:11:12");
    assert_eq!(body["minutes_east"], 480);
    assert_eq!(body["before"]["time_zone"]["timeMin"], -420);
    let zone = frames_named(&device, msg::CONFIG_SET, "System.TimeZone");
    assert_eq!(
        zone[0]["System.TimeZone"],
        json!({ "FirstUserTimeZone": 0, "timeMin": -480 })
    );
    let time = frames_named(&device, msg::OP_MACHINE, "OPTimeSetting");
    assert_eq!(time[0]["OPTimeSetting"], "2026-10-30 10:11:12");
    let backup = body["backup"].as_str().unwrap();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(backup).unwrap()).unwrap();
    assert_eq!(saved["time_zone"]["timeMin"], -420);
}

#[tokio::test]
async fn language_write_refuses_values_outside_the_device_list() {
    env();
    let (address, device) = fake(Mood::Talking).await;
    let client = client(&address).await;
    let (status, body) = call(
        &client,
        "PUT",
        "/api/device/system/language?value=Klingon",
        None,
    )
    .await;
    assert_eq!(status, Status::BadRequest, "{body}");
    assert!(frames_named(&device, msg::CONFIG_SET, "General.Location.Language").is_empty());
    let (status, body) = call(
        &client,
        "PUT",
        "/api/device/system/language?value=Indonesian",
        None,
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["current"], "Indonesian");
    assert_eq!(body["before"], "English");
    let set = frames_named(&device, msg::CONFIG_SET, "General.Location.Language");
    assert_eq!(set[0]["General.Location.Language"], "Indonesian");
}

#[tokio::test]
async fn users_never_return_the_hash_and_password_ops_send_the_app_wire() {
    env();
    let (address, device) = fake(Mood::Talking).await;
    let client = client(&address).await;
    let (status, body) = call(&client, "GET", "/api/device/system/users", None).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["users"][0]["name"], "admin");
    assert!(!body.to_string().contains("tlJwpbo6"));
    assert_eq!(body["recovery"]["Question"], 1);
    let (status, body) = call(
        &client,
        "POST",
        "/api/device/system/users/password",
        Some(json!({ "user": "admin", "old": "old-pass", "new": "new-pass" })),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let change = device
        .lock()
        .unwrap()
        .frames
        .iter()
        .find(|(id, _, _)| *id == msg::MODIFY_PASSWORD)
        .map(|(_, body, _)| body.clone())
        .unwrap();
    assert_eq!(change["EncryptType"], "MD5");
    assert_eq!(change["PassWord"], sofia_hash("old-pass"));
    assert_eq!(change["NewPassWord"], sofia_hash("new-pass"));
    assert_eq!(change["UserName"], "admin");
    let (status, body) = call(
        &client,
        "POST",
        "/api/device/system/users/rename",
        Some(json!({ "user": "admin", "name": "keeper" })),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let rename = frames_named(&device, msg::MODIFY_USER, "ModifyUser");
    assert_eq!(rename[0]["UserName"], "admin");
    assert_eq!(rename[0]["User"]["Name"], "keeper");
    assert_eq!(rename[0]["User"]["Group"], "admin");
    let (status, body) = call(
        &client,
        "POST",
        "/api/device/system/users/recover",
        Some(json!({ "step": "password", "user": "admin", "password": "fresh" })),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let recover = frames_named(&device, msg::SAFETY, "SetNewPassword");
    assert_eq!(
        recover[0]["SetNewPassword"],
        json!({ "UserName": "admin", "NewPassword": "fresh" })
    );
    let (status, body) = call(
        &client,
        "POST",
        "/api/device/system/users/recover",
        Some(json!({ "step": "answer", "answers": ["a", "b"] })),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let answer = frames_named(&device, msg::SAFETY, "CheckSafetyAnswer");
    assert_eq!(
        answer[0]["CheckSafetyAnswer"],
        json!({ "Answer": ["a", "b"] })
    );
}

#[tokio::test]
async fn firmware_start_and_file_upload_need_the_word_and_use_2018_and_1520() {
    env();
    let (address, device) = fake(Mood::Talking).await;
    let client = client(&address).await;
    let (status, _) = call(
        &client,
        "POST",
        "/api/device/system/firmware/start",
        Some(json!({ "confirm": "upgrade", "file_name": "fw.bin", "version_date": "2026-01-02" })),
    )
    .await;
    assert_eq!(status, Status::BadRequest);
    assert!(!ids(&device).contains(&msg::START_UPGRADE));
    let (status, body) = call(
        &client,
        "POST",
        "/api/device/system/firmware/start",
        Some(json!({ "confirm": "UPGRADE", "file_name": "fw.bin", "version_date": "2026-01-02" })),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let start = frames_named(&device, msg::START_UPGRADE, "OPStartUpgradeReq");
    assert_eq!(
        start[0]["OPStartUpgradeReq"],
        json!({ "UUID": "serial-one", "DevID": "ABCDEF01100101234560000000", "FileName": "fw.bin", "VerDate": "2026-01-02", "Manual": 1 })
    );
    let image = vec![9u8; 40_000];
    let response = tokio::time::timeout(
        OUTER,
        client
            .post("/api/device/system/firmware/file?confirm=UPGRADE")
            .header(ContentType::Binary)
            .body(image.clone())
            .dispatch(),
    )
    .await
    .expect("upload answered");
    assert_eq!(response.status(), Status::Ok);
    let frames = device.lock().unwrap().frames.clone();
    let upgrade = frames
        .iter()
        .find(|(id, _, _)| *id == msg::SYSTEM_UPGRADE)
        .unwrap();
    assert_eq!(
        upgrade.1["OPSystemUpgrade"],
        json!({ "Action": "Start", "Type": "System", "FileSize": 40_000 })
    );
    let data: Vec<usize> = frames
        .iter()
        .filter(|(id, _, _)| *id == msg::UPGRADE_DATA)
        .map(|(_, _, length)| *length)
        .collect();
    assert_eq!(data, vec![32 * 1024, 40_000 - 32 * 1024, 0]);
}

#[tokio::test]
async fn firmware_check_asks_the_device_and_the_upgrade_server() {
    env();
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", http.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(String::new()));
    let record = seen.clone();
    tokio::spawn(async move {
        let (mut socket, _) = http.accept().await.unwrap();
        let mut buffer = vec![0u8; 8192];
        let mut raw = Vec::new();
        loop {
            let read = socket.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&buffer[..read]);
            let text = String::from_utf8_lossy(&raw).to_string();
            if let Some(split) = text.find("\r\n\r\n") {
                let length = text
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if raw.len() >= split + 4 + length {
                    break;
                }
            }
        }
        *record.lock().unwrap() = String::from_utf8_lossy(&raw).to_string();
        socket
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    std::env::set_var("PROVIEW_UPGRADE_BASE", &base);
    let (address, device) = fake(Mood::Talking).await;
    let client = client(&address).await;
    let (status, body) = call(&client, "GET", "/api/device/system/firmware", None).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["dev_id"], "ABCDEF01100101234560000000");
    assert_eq!(body["build_date"], "2024-05-16");
    assert_eq!(body["server_check"]["up_to_date"], true);
    assert_eq!(body["server_check"]["http_status"], 204);
    assert!(body["device_check"]["OPVersionRep"].is_object(), "{body}");
    let version = frames_named(&device, msg::VERSION_REQ, "OPVersionReq");
    assert_eq!(
        version[0]["OPVersionReq"],
        json!({ "Expect": 0, "Language": "English", "Manual": 1 })
    );
    let request = seen.lock().unwrap().clone();
    assert!(request.starts_with("POST /list HTTP/1.1"), "{request}");
    let sent: Value =
        serde_json::from_str(&request[request.find("\r\n\r\n").unwrap() + 4..]).unwrap();
    assert_eq!(sent["DevID"], "ABCDEF01100101234560000000");
    assert_eq!(sent["CurVersion"], "2024-05-16");
    assert_eq!(sent["DevType"], "IPC");
    assert_eq!(sent["SN"], "serial-one");
    assert_eq!(sent["PID"], "PID-ONE");
}

#[tokio::test]
async fn network_check_pings_the_app_hosts_plus_gateway_and_gates_wifi() {
    env();
    let (address, device) = fake(Mood::Talking).await;
    let client = client(&address).await;
    let (status, body) = call(&client, "GET", "/api/device/system/network", None).await;
    assert_eq!(status, Status::Ok, "{body}");
    let urls: Vec<String> = body["pings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ping| ping["url"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        urls,
        vec![
            "223.5.5.5",
            "8.8.8.8",
            "secu100.net",
            "www.microsoft.com",
            "192.168.1.1"
        ]
    );
    assert_eq!(body["pings"][0]["reachable"], true);
    assert_eq!(body["pings"][0]["lost_percent"], 0);
    assert_eq!(body["pings"][0]["sent"], 3);
    assert_eq!(body["cloud_status"]["Status"], "Connected");
    assert_eq!(body["nat"]["NatStatus"], "Conneted");
    assert!(body["wifi"].is_null());
    assert!(body["wifi_error"]
        .as_str()
        .unwrap()
        .contains("WifiRouteSignalLevel"));
    assert!(frames_named(&device, msg::SYSTEM_INFO, "WifiRouteInfo").is_empty());
    let ping = frames_named(&device, msg::PING, "Ping");
    assert_eq!(
        ping[0]["Ping"],
        json!({ "Num": 3, "Timeout": 2, "URL": "223.5.5.5" })
    );
}
