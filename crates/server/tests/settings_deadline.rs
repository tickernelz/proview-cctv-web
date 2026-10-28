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

const WINDOW_MS: &str = "400";
const OUTER: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Everything,
    ConfigGetSilent,
    LoginSilent,
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

fn settings_env() {
    std::env::set_var("PROVIEW_SETTINGS_WINDOW_MS", WINDOW_MS);
    std::env::set_var(
        "PROVIEW_SETTINGS_BACKUP_DIR",
        std::env::temp_dir().join("pvw-settings-deadline"),
    );
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

fn trim(payload: &[u8]) -> &[u8] {
    let mut end = payload.len();
    while end > 0 && matches!(payload[end - 1], 0 | b'\n' | b'\r') {
        end -= 1;
    }
    &payload[..end]
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
        let raw = trim(&body);
        let requested = serde_json::from_slice::<Value>(raw).ok().and_then(|value| {
            value
                .get("Name")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        let answer_body = match (answer, header.id) {
            (Answer::LoginSilent, _) => None,
            (Answer::ConfigGetSilent, msg::CONFIG_GET) => None,
            (_, msg::LOGIN) => Some(json!({ "Ret": 100, "SessionID": "0x11223344" })),
            (_, msg::CONFIG_GET | msg::SYSTEM_INFO) => {
                let mut value = json!({ "Ret": 100 });
                if let Some(name) = requested {
                    value[name] = json!([{}]);
                }
                Some(value)
            }
            _ => Some(json!({ "Ret": 100 })),
        };
        let Some(value) = answer_body else {
            std::future::pending::<()>().await;
            return;
        };
        let out = reply(&header, &value);
        if socket.write_all(&out).await.is_err() {
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

async fn read(client: &Client, path: &str) -> (Status, String) {
    let response = tokio::time::timeout(OUTER, client.get(path).dispatch())
        .await
        .unwrap_or_else(|_| panic!("{path} never answered inside {OUTER:?}"));
    let status = response.status();
    (status, response.into_string().await.unwrap_or_default())
}

const READS: [&str; 3] = [
    "/api/device/settings/encode",
    "/api/device/settings/network",
    "/api/device/settings/storage",
];

#[tokio::test]
async fn the_three_reads_answer_from_a_device_that_keeps_talking() {
    settings_env();
    let client = client(&fake(Answer::Everything).await).await;
    for path in READS {
        let (status, body) = read(&client, path).await;
        assert_eq!(status, Status::Ok, "{path}: {body}");
    }
}

#[tokio::test]
async fn a_device_that_stays_silent_on_config_get_times_the_read_out() {
    settings_env();
    let client = client(&fake(Answer::ConfigGetSilent).await).await;
    for path in READS {
        let (status, body) = read(&client, path).await;
        assert_eq!(status, Status::GatewayTimeout, "{path}: {body}");
        assert!(
            body.contains("did not answer inside the settings window"),
            "{path}: {body}"
        );
    }
}

#[tokio::test]
async fn a_device_that_stays_silent_on_login_times_the_session_out() {
    settings_env();
    let client = client(&fake(Answer::LoginSilent).await).await;
    for path in READS {
        let (status, body) = read(&client, path).await;
        assert_eq!(status, Status::GatewayTimeout, "{path}: {body}");
        assert!(body.contains("login"), "{path}: {body}");
    }
}
