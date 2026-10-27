use std::path::PathBuf;
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

const WINDOW_MS: &str = "300";
const OUTER: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Everything,
    SilentAfterLogin,
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
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(serve(socket, answer));
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

async fn serve(mut socket: TcpStream, answer: Answer) {
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
        let out = match answer {
            Answer::Everything => talking_reply(&header, name.as_deref()),
            Answer::SilentAfterLogin if header.id == msg::LOGIN => {
                reply(&header, &json!({ "Ret": 100, "SessionID": "0x11223344" }))
            }
            Answer::SilentAfterLogin => {
                std::future::pending::<()>().await;
                return;
            }
        };
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
