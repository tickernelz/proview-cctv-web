use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener as StdListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use proview_core::provider::{Mode, VendorSession};
use proview_core::session::Store;
use proview_server::config::ModeStore;
use rocket::http::{Cookie, CookieJar};
use rocket::post;
use rocket::routes;
use rocket::State;

const OWN_SERIAL: &str = "7f92970ddf5282743hrc";
const OTHER_SERIAL: &str = "00000000000000000001";

type Rows = Arc<Mutex<Vec<String>>>;
type Stop = Arc<std::sync::atomic::AtomicBool>;
type Hits = Arc<std::sync::atomic::AtomicUsize>;

fn fake_cloud(rows: Rows, hits: Hits, stop: Stop) -> (String, std::thread::JoinHandle<()>) {
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
            if request_line.contains("msdlistCode") {
                hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

#[post("/_test/session/<who>")]
fn seed_session(store: &State<Store>, cookies: &CookieJar<'_>, who: String) -> String {
    let session = proview_core::session::new_id();
    store.put(
        session.clone(),
        VendorSession {
            account: format!("{who}@example.invalid"),
            token: Some(format!("token-{who}")),
        },
        Duration::from_secs(600),
    );
    cookies.add_private(Cookie::build(("pvw_session", session.clone())).build());
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
    let config = config(cloud_base);
    let cloud = Arc::new(proview_core::provider::cloud::CloudProvider::new(&config));
    let rig = rocket::custom(figment)
        .manage(config)
        .manage(cloud)
        .manage(Store::new())
        .manage(ModeStore::new(Mode::Cloud, None))
        .manage(proview_server::live_state::LiveState::new())
        .mount(
            "/api",
            routes![
                proview_server::routes::ws_live,
                proview_server::routes::live::stream,
                seed_session
            ],
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

fn read_http(stream: &mut TcpStream, want_body: bool) -> (u16, Vec<String>, String) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut status_line = String::new();
    reader.read_line(&mut status_line).expect("status line");
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
        headers.push(line.trim().to_string());
    }
    let mut body = String::new();
    if want_body {
        let length = headers
            .iter()
            .find_map(|line| {
                let lower = line.to_ascii_lowercase();
                lower
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        let mut bytes = vec![0u8; length];
        let _ = reader.read_exact(&mut bytes);
        body = String::from_utf8_lossy(&bytes).into_owned();
    }
    (code, headers, body)
}

fn open_session(address: SocketAddr, who: &str) -> String {
    let mut stream = TcpStream::connect(address).expect("connect");
    let request = format!(
        "POST /api/_test/session/{who} HTTP/1.1\r\nHost: {address}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).expect("write");
    let (code, headers, _) = read_http(&mut stream, false);
    assert_eq!(code, 200, "session seed for {who}");
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

fn ws_probe(address: SocketAddr, serial: &str, cookie: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    let request = format!(
        "GET /api/ws/live?device={serial}&channel=1&quality=main HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nCookie: {cookie}\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).expect("write");
    let (code, _, body) = read_http(&mut stream, true);
    (code, body)
}

#[test]
fn a_session_cannot_open_the_socket_of_a_serial_it_does_not_own() {
    let rows: Rows = Arc::new(Mutex::new(vec![OTHER_SERIAL.to_string()]));
    let hits: Hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stop: Stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (base, cloud) = fake_cloud(rows, hits, stop.clone());
    let (address, runtime) = spawn_server(base);
    let cookie = open_session(address, "b");

    let (foreign_code, foreign_body) = ws_probe(address, OWN_SERIAL, &cookie);
    assert_eq!(
        foreign_code, 404,
        "B must not open A's camera: {foreign_body}"
    );
    assert!(
        foreign_body.contains("device not in this account"),
        "the refusal must name the reason: {foreign_body}"
    );

    let (own_code, own_body) = ws_probe(address, OTHER_SERIAL, &cookie);
    assert_eq!(
        own_code, 101,
        "B owns the serial its account lists: {own_body}"
    );

    drop(cookie);
    drop(runtime);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    cloud.join().ok();
}

fn get_probe(address: SocketAddr, path: &str, cookie: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    let request = format!(
        "GET /api{path} HTTP/1.1\r\nHost: {address}\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).expect("write");
    let (code, _, body) = read_http(&mut stream, true);
    (code, body)
}

fn ws_status(address: SocketAddr, serial: &str, cookie: &str) -> u16 {
    let mut stream = TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let request = format!(
        "GET /api/ws/live?device={serial}&channel=1&quality=main HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nCookie: {cookie}\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).expect("write");
    let (code, _, _) = read_http(&mut stream, false);
    code
}

#[test]
fn one_viewer_open_asks_the_device_list_once() {
    let rows: Rows = Arc::new(Mutex::new(vec![OWN_SERIAL.to_string()]));
    let hits: Hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stop: Stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (base, cloud) = fake_cloud(rows, hits.clone(), stop.clone());
    let (address, runtime) = spawn_server(base);
    let cookie = open_session(address, "a");

    let (ticket, body) = get_probe(
        address,
        &format!("/stream?device={OWN_SERIAL}&channel=1&quality=main"),
        &cookie,
    );
    assert!(
        ticket == 200 || ticket == 501,
        "the ticket answered {ticket}: {body}"
    );
    for attempt in 1..=2 {
        let code = ws_status(address, OWN_SERIAL, &cookie);
        assert!(
            code == 101 || code == 200,
            "attempt {attempt} opened the own camera: {code}"
        );
    }
    assert_eq!(
        hits.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "one viewer open must pay one device list, not one per call"
    );

    drop(runtime);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    cloud.join().ok();
}
