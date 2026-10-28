use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener as StdListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use proview_core::provider::{Mode, VendorSession};
use proview_core::session::Store;
use proview_server::config::ModeStore;
use rocket::http::CookieJar;
use rocket::post;
use rocket::routes;
use rocket::State;

const LISTED: &str = "serial-review-0001";
const OTHER: &str = "serial-other-0002";
const FOREIGN: &str = "serial-foreign-0003";

type Rows = Arc<Mutex<Vec<String>>>;
type Stop = Arc<std::sync::atomic::AtomicBool>;

fn fake_cloud(rows: Rows, stop: Stop) -> (String, std::thread::JoinHandle<()>) {
    let listener = StdListener::bind("127.0.0.1:0").expect("fake cloud listener");
    let address = listener.local_addr().expect("fake cloud address");
    listener.set_nonblocking(true).expect("non blocking");
    let handle = std::thread::spawn(move || {
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            let stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
            };
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let Ok(mut writer) = stream.try_clone() else {
                continue;
            };
            let mut reader = BufReader::new(stream);
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; length];
            if length > 0 && reader.read_exact(&mut body).is_err() {
                continue;
            }
            let serials = rows.lock().expect("rows").clone();
            let entries: Vec<String> = serials
                .iter()
                .map(|serial| {
                    format!(
                        "{{\"uuid\":\"{serial}\",\"nickname\":\"camera\",\"username\":\"admin\",\"deviceToken\":{{\"AdminToken\":\"token\"}}}}"
                    )
                })
                .collect();
            let payload = format!(
                "{{\"code\":2000,\"msg\":\"SUCCESS\",\"data\":{{\"mine\":[{}]}}}}",
                entries.join(",")
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            let _ = writer.write_all(response.as_bytes());
        }
    });
    (format!("http://{address}"), handle)
}

fn config(cloud_base: String) -> proview_core::config::Config {
    proview_core::config::Config {
        provider: proview_core::config::ProviderKind::Proview,
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

#[post("/_test/session")]
fn seed_session(store: &State<Store>, cookies: &CookieJar<'_>) -> String {
    let session = proview_core::session::new_id();
    store.put(
        session.clone(),
        VendorSession {
            account: "probe@example.invalid".to_string(),
            token: Some("token-probe".to_string()),
        },
        Duration::from_secs(600),
    );
    cookies.add_private(rocket::http::Cookie::build(("pvw_session", session.clone())).build());
    session
}

fn free_port() -> u16 {
    let probe = StdListener::bind("127.0.0.1:0").expect("probe listener");
    probe.local_addr().expect("probe address").port()
}

fn spawn_server(cloud_base: String) -> (SocketAddr, tokio::runtime::Runtime) {
    let port = free_port();
    let figment = rocket::Config::figment()
        .merge(("address", "127.0.0.1"))
        .merge(("port", port));
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let rig = rocket::custom(figment)
        .manage(config(cloud_base))
        .manage(Store::new())
        .manage(ModeStore::new(Mode::Cloud, None))
        .manage(proview_server::live_state::LiveState::new())
        .mount(
            "/api",
            routes![proview_server::routes::live::stream, seed_session],
        );
    let server = runtime.block_on(async { rig.ignite().await.expect("ignite") });
    runtime.spawn(async move {
        let _ = server.launch().await;
    });
    let address: SocketAddr = format!("127.0.0.1:{port}").parse().expect("address");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if TcpStream::connect(address).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    (address, runtime)
}

fn read_http(stream: &mut TcpStream) -> (u16, String) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut status_line = String::new();
    reader.read_line(&mut status_line).expect("status line");
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; length];
    let _ = reader.read_exact(&mut body);
    (code, String::from_utf8_lossy(&body).into_owned())
}

fn signed_in(address: SocketAddr) -> String {
    let mut stream = TcpStream::connect(address).expect("connect");
    let request = format!(
        "POST /api/_test/session HTTP/1.1\r\nHost: {address}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).expect("write");
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut status_line = String::new();
    reader.read_line(&mut status_line).expect("status line");
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
        headers.push(line.trim().to_string());
    }
    headers
        .iter()
        .find(|line| line.to_ascii_lowercase().starts_with("set-cookie:"))
        .map(|line| {
            line[11..]
                .trim()
                .split(';')
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .expect("session cookie")
}

fn ticket(address: SocketAddr, serial: &str, cookie: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    let request = format!(
        "GET /api/stream?device={serial}&channel=1&quality=main HTTP/1.1\r\nHost: {address}\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).expect("write");
    read_http(&mut stream)
}

fn set_env(serial: Option<&str>, user: Option<&str>, pass: Option<&str>) {
    for (key, value) in [
        ("PROVIEW_DEVICE_SERIAL", serial),
        ("PROVIEW_DEVICE_USER", user),
        ("PROVIEW_DEVICE_PASS", pass),
    ] {
        match value {
            Some(text) => std::env::set_var(key, text),
            None => std::env::remove_var(key),
        }
    }
}

#[test]
fn the_ticket_names_why_the_device_cannot_log_in() {
    let rows: Rows = Arc::new(Mutex::new(vec![LISTED.to_string()]));
    let stop: Stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (base, cloud) = fake_cloud(rows, stop.clone());
    let (address, runtime) = spawn_server(base);
    let cookie = signed_in(address);

    set_env(None, Some("admin"), Some("camera-pass"));
    let (code, body) = ticket(address, LISTED, &cookie);
    assert_eq!(
        code, 501,
        "an unset serial is a configuration answer: {body}"
    );
    assert!(body.contains("PROVIEW_DEVICE_SERIAL"), "{body}");
    assert!(body.contains("misconfigured"), "{body}");

    set_env(Some(OTHER), Some("admin"), Some("camera-pass"));
    let (code, body) = ticket(address, LISTED, &cookie);
    assert_eq!(
        code, 501,
        "another serial is a configuration answer: {body}"
    );
    assert!(body.contains("belong to"), "{body}");

    set_env(Some(LISTED), Some("admin"), Some("camera-pass"));
    let (code, body) = ticket(address, LISTED, &cookie);
    assert_eq!(code, 200, "a configured serial issues a ticket: {body}");
    assert!(body.contains("/ws/live"), "{body}");

    set_env(Some(LISTED), Some("admin"), Some("camera-pass"));
    let (code, body) = ticket(address, FOREIGN, &cookie);
    assert_eq!(
        code, 404,
        "a serial the account does not list is refused: {body}"
    );

    set_env(None, None, None);
    drop(runtime);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    cloud.join().ok();
}
