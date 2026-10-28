use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener as StdListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use proview_core::cloud::Identity;
use proview_core::provider::Mode;
use proview_server::config::ModeStore;
use proview_server::settings::SettingsStore;
use rocket::http::{ContentType, Status};
use rocket::local::blocking::Client;
use serde_json::json;

const SAVED: [(&str, &str); 3] = [
    ("PROVIEW_APP_UUID", "uuid-saved-at-runtime"),
    ("PROVIEW_APP_KEY", "key-saved-at-runtime"),
    ("PROVIEW_APP_SECRET", "secret-saved-at-runtime"),
];

type Hits = Arc<AtomicUsize>;
type SeenUuid = Arc<std::sync::Mutex<String>>;

fn fake_cloud() -> (String, Hits, SeenUuid, Arc<std::sync::atomic::AtomicBool>) {
    let listener = StdListener::bind("127.0.0.1:0").expect("fake cloud listener");
    let address = listener.local_addr().expect("fake cloud address");
    listener.set_nonblocking(true).expect("non blocking");
    let hits: Hits = Arc::new(AtomicUsize::new(0));
    let seen: SeenUuid = Arc::new(std::sync::Mutex::new(String::new()));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let counted = hits.clone();
    let recorded = seen.clone();
    let ending = stop.clone();
    std::thread::spawn(move || {
        while !ending.load(Ordering::Relaxed) {
            let (stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
            };
            serve(stream, &counted, &recorded);
        }
    });
    (format!("http://{address}"), hits, seen, stop)
}

fn serve(stream: TcpStream, hits: &AtomicUsize, seen: &SeenUuid) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut length = 0usize;
    let mut uuid = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("uuid:") {
            uuid = line[5..].trim().to_string();
        }
    }
    let mut body = vec![0u8; length];
    if length > 0 && reader.read_exact(&mut body).is_err() {
        return;
    }
    let payload = if request_line.contains("loginExist") {
        hits.fetch_add(1, Ordering::Relaxed);
        *seen.lock().expect("seen") = uuid;
        "{\"code\":2000,\"msg\":\"SUCCESS\",\"data\":{\"accessToken\":\"access\",\"refreshToken\":\"refresh\"}}"
    } else {
        "{\"code\":2000,\"msg\":\"SUCCESS\",\"data\":{}}"
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        payload.len(),
        payload
    );
    let _ = writer.write_all(response.as_bytes());
}

fn config(cloud_base: String) -> proview_core::config::Config {
    proview_core::config::Config {
        provider: proview_core::config::ProviderKind::Proview,
        share_base: cloud_base.clone(),
        cloud_base,
        device_user: "admin".to_string(),
        device_pass: String::new(),
        fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
        device_host: None,
        device_onvif: None,
        device_rtsp: None,
        session_ttl_secs: 3600,
    }
}

fn figment() -> rocket::figment::Figment {
    rocket::Config::figment()
        .merge(("secret_key", "uVo3haa/znSxx4MPMV95m4oe9EYQe99QbYgEuBtVqqU="))
        .merge(("port", 0))
        .merge(("log_level", "critical"))
}

fn sign_in(client: &Client) -> (Status, String) {
    let response = client
        .post("/api/login")
        .header(ContentType::JSON)
        .body(json!({"email": "probe@example.invalid", "password": "secret"}).to_string())
        .dispatch();
    let status = response.status();
    let body = response.into_string().unwrap_or_default();
    (status, body)
}

#[test]
fn the_platform_identity_is_read_per_request_not_frozen_at_start() {
    for (key, _) in SAVED {
        std::env::remove_var(key);
    }
    let dir = std::env::temp_dir().join(format!("pvw-identity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");

    let (base, hits, seen, stop) = fake_cloud();
    let settings = SettingsStore::load(Some(&dir));
    settings.apply_env();
    let client = Client::tracked(proview_server::build_with_state(
        figment(),
        config(base),
        ModeStore::new(Mode::Cloud, None),
        settings,
    ))
    .expect("valid rocket instance");

    if !Identity::built().is_complete() {
        let (status, body) = sign_in(&client);
        assert_eq!(status, Status::NotImplemented, "{body}");
        assert!(body.contains("cloud identity"), "{body}");
        assert_eq!(
            hits.load(Ordering::Relaxed),
            0,
            "no identity, no vendor call"
        );
    }

    let values: serde_json::Map<String, serde_json::Value> = SAVED
        .iter()
        .map(|(key, value)| (key.to_string(), json!(value)))
        .collect();
    let saved = client
        .post("/api/settings")
        .header(ContentType::JSON)
        .body(json!({"values": values}).to_string())
        .dispatch();
    assert_eq!(saved.status(), Status::Ok, "the settings save answered");

    let (status, body) = sign_in(&client);
    assert_eq!(
        status,
        Status::Ok,
        "an identity saved after start must reach the vendor: {body}"
    );
    assert_eq!(
        hits.load(Ordering::Relaxed),
        1,
        "the saved identity made the login call"
    );
    assert_eq!(
        seen.lock().expect("seen").as_str(),
        "uuid-saved-at-runtime",
        "the login must be signed with the identity saved after start, not the built-in one"
    );

    for (key, _) in SAVED {
        std::env::remove_var(key);
    }
    stop.store(true, Ordering::Relaxed);
    let _ = std::fs::remove_dir_all(dir);
}
