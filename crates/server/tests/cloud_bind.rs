use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Once;

use proview_core::cloud::{body_key, open, seal};
use proview_core::config::{Config, ProviderKind};
use proview_server::{serve_with_state, ServeOptions};
use reqwest::{Client, Method, Response};
use serde_json::{json, Value};

const UUID: &str = "000000000000000000000001";
const APP_KEY: &str = "test-app-key";
const APP_SECRET: &str = "0123456789abcdef0123456789abcdef";
const ACCOUNT: &str = "acct-0000000000000000000001";

static ENV: Once = Once::new();

fn prepare_env() {
    ENV.call_once(|| {
        std::env::set_var("PROVIEW_APP_UUID", UUID);
        std::env::set_var("PROVIEW_APP_KEY", APP_KEY);
        std::env::set_var("PROVIEW_APP_SECRET", APP_SECRET);
    });
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or_default()
}

fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18) as usize & 63] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(triple >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[triple as usize & 63] as char);
        }
    }
    out
}

fn token(expires_in: i64) -> String {
    let header = base64url(br#"{"typ":"JWT","alg":"HS256"}"#);
    let payload = base64url(
        format!(
            r#"{{"iss":"rs","appKey":"{APP_KEY}","account":"{ACCOUNT}","exp":{}}}"#,
            now() + expires_in
        )
        .as_bytes(),
    );
    format!("{header}.{payload}.signature")
}

fn reply(method: &str, form: &str) -> Value {
    match method {
        "loginExist" => json!({
            "code": 2000,
            "msg": "SUCCESS",
            "data": { "accessToken": token(3600), "refreshToken": "refresh" }
        }),
        "msdlistCode" => json!({
            "code": 2000,
            "msg": "SUCCESS",
            "data": { "mine": [ { "uuid": "AASN000000000001", "nickname": "Gate" } ], "share": [] }
        }),
        "mdadd" => {
            let serial = form
                .split('&')
                .find_map(|pair| pair.strip_prefix("uuid="))
                .unwrap_or_default();
            match serial {
                "AASN000000000001" => json!({ "code": 4101, "msg": "DEV_ALREADY", "data": "" }),
                "AASN000000000003" => {
                    json!({ "code": 4105, "msg": "DEV_USERNAME_FAIL", "data": "" })
                }
                "AASN000000000004" => json!({ "code": -4004, "msg": "token expired", "data": "" }),
                _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
            }
        }
        "mdrm" => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
    }
}

fn find_head(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
}

fn cloud_stub() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("stub listener");
    let address = listener.local_addr().expect("stub address");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(stream) => stream,
                Err(_) => continue,
            };
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let read = match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(head) = find_head(&buffer) {
                    let head_text = String::from_utf8_lossy(&buffer[..head]).to_string();
                    if buffer.len() >= head + 4 + content_length(&head_text) {
                        break;
                    }
                }
            }
            let head = find_head(&buffer).unwrap_or(buffer.len());
            let head_text = String::from_utf8_lossy(&buffer[..head]).to_string();
            let request = head_text.lines().next().unwrap_or_default().to_string();
            let path = request.split(' ').nth(1).unwrap_or("/").to_string();
            let method = path
                .trim_start_matches('/')
                .split('/')
                .next()
                .unwrap_or_default()
                .to_string();
            let tsid = path.split('/').nth(3).unwrap_or_default().to_string();
            let length = content_length(&head_text);
            let body = if buffer.len() >= head + 4 {
                String::from_utf8_lossy(&buffer[head + 4..(head + 4 + length).min(buffer.len())])
                    .trim()
                    .to_string()
            } else {
                String::new()
            };
            let key = body_key(&tsid, APP_SECRET);
            let form = if body.is_empty() {
                String::new()
            } else {
                open(&key, &body).unwrap_or_default()
            };
            let payload = reply(&method, &form).to_string();
            let sealed = seal(&key, &payload).unwrap_or(payload);
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                sealed.len(),
                sealed
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{address}")
}

struct Browser {
    http: Client,
    cookies: BTreeMap<String, String>,
}

impl Browser {
    fn new() -> Self {
        Self {
            http: Client::builder().build().expect("http client"),
            cookies: BTreeMap::new(),
        }
    }

    async fn send(&mut self, method: Method, url: &str, body: Option<Value>) -> (u16, Value) {
        let mut request = self.http.request(method, url);
        if !self.cookies.is_empty() {
            let header = self
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");
            request = request.header("cookie", header);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response: Response = request.send().await.expect("request");
        let status = response.status().as_u16();
        for value in response.headers().get_all("set-cookie") {
            let text = value.to_str().unwrap_or_default().to_string();
            let first = text.split(';').next().unwrap_or_default();
            if let Some((name, value)) = first.split_once('=') {
                let name = name.trim().to_string();
                let value = value.trim().to_string();
                let expired = text.to_ascii_lowercase().contains("max-age=0") || value.is_empty();
                if expired {
                    self.cookies.remove(&name);
                } else {
                    self.cookies.insert(name, value);
                }
            }
        }
        let text = response.text().await.unwrap_or_default();
        let parsed = serde_json::from_str(&text).unwrap_or(Value::String(text));
        (status, parsed)
    }

    async fn get(&mut self, base: &str, path: &str) -> (u16, Value) {
        self.send(Method::GET, &format!("{base}{path}"), None).await
    }

    async fn post(&mut self, base: &str, path: &str, body: Value) -> (u16, Value) {
        self.send(Method::POST, &format!("{base}{path}"), Some(body))
            .await
    }

    fn remember_secret(&self) -> Option<String> {
        self.cookies.get("pvw_remember").cloned()
    }

    fn set_cookie(&mut self, name: &str, value: &str) {
        self.cookies.insert(name.to_string(), value.to_string());
    }
}

fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pvw-bind-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("data dir");
    dir
}

struct Running {
    base: String,
    shutdown: rocket::Shutdown,
}

async fn start(dir: &Path, cloud: &str) -> Running {
    prepare_env();
    let config = Config {
        provider: ProviderKind::Proview,
        cloud_base: cloud.to_string(),
        device_user: "admin".to_string(),
        device_pass: "device-password".to_string(),
        fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
        device_host: None,
        device_onvif: None,
        device_rtsp: None,
        session_ttl_secs: 3600,
    };
    let settings = proview_server::settings::SettingsStore::load(Some(dir));
    let opts = ServeOptions {
        address: "127.0.0.1".to_string(),
        port: 0,
        secret_key: Some("uVo3haa/znSxx4MPMV95m4oe9EYQe99QbYgEuBtVqqU=".to_string()),
        data_dir: Some(dir.to_path_buf()),
    };
    let handle = serve_with_state(opts, config, settings)
        .await
        .expect("server");
    let base = format!("http://{}", handle.address);
    wait_ready(&base).await;
    Running {
        base,
        shutdown: handle.shutdown,
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

async fn wait_ready(base: &str) {
    let client = Client::new();
    for _ in 0..100 {
        if let Ok(response) = client.get(format!("{base}/api/health")).send().await {
            if response.status().is_success() {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the server did not answer on {base}");
}

async fn sign_in(browser: &mut Browser, base: &str) -> u16 {
    browser
        .post(
            base,
            "/api/login",
            json!({ "email": "someone@example.invalid", "password": "secret" }),
        )
        .await
        .0
}

async fn remember(browser: &mut Browser, base: &str) -> Option<String> {
    let (status, _) = browser.post(base, "/api/account/remember", json!({})).await;
    if status != 200 {
        return None;
    }
    browser.remember_secret()
}

#[test]
fn without_the_browser_cookie_the_remembered_account_restores_nothing() {
    block_on(async {
        let dir = data_dir("anon");
        let cloud = cloud_stub();
        let running = start(&dir, &cloud).await;
        let mut owner = Browser::new();
        assert_eq!(sign_in(&mut owner, &running.base).await, 200);
        assert!(remember(&mut owner, &running.base).await.is_some());

        let mut stranger = Browser::new();
        let (status, reply) = stranger.get(&running.base, "/api/account/remembered").await;
        assert_eq!(status, 200);
        assert_eq!(reply["restored"], json!(false));
        assert_eq!(reply["email"], json!(""));
        assert!(stranger.remember_secret().is_none());
        assert_eq!(stranger.get(&running.base, "/api/devices").await.0, 401);

        let mut guessing = Browser::new();
        guessing.set_cookie("pvw_remember", "a-different-browser");
        let (_, reply) = guessing.get(&running.base, "/api/account/remembered").await;
        assert_eq!(reply["restored"], json!(false));
        running.shutdown.notify();
    });
}

#[test]
fn remember_needs_a_session() {
    block_on(async {
        let dir = data_dir("session");
        let cloud = cloud_stub();
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        let (status, _) = browser
            .post(&running.base, "/api/account/remember", json!({}))
            .await;
        assert_eq!(status, 401);
        running.shutdown.notify();
    });
}

#[test]
fn only_the_browser_that_remembered_gets_the_session_back() {
    block_on(async {
        let dir = data_dir("restore");
        let cloud = cloud_stub();
        let running = start(&dir, &cloud).await;
        let mut owner = Browser::new();
        assert_eq!(sign_in(&mut owner, &running.base).await, 200);
        let secret = remember(&mut owner, &running.base).await.expect("secret");

        let mut restoring = Browser::new();
        restoring.set_cookie("pvw_remember", &secret);
        let (_, reply) = restoring
            .get(&running.base, "/api/account/remembered")
            .await;
        assert_eq!(reply["restored"], json!(true));
        assert_eq!(reply["email"], json!("someone@example.invalid"));
        assert_eq!(restoring.get(&running.base, "/api/devices").await.0, 200);
        running.shutdown.notify();
    });
}

#[test]
fn sign_out_revokes_every_session_that_carries_the_same_token() {
    block_on(async {
        let dir = data_dir("revoke");
        let cloud = cloud_stub();
        let running = start(&dir, &cloud).await;
        let mut owner = Browser::new();
        assert_eq!(sign_in(&mut owner, &running.base).await, 200);
        let secret = remember(&mut owner, &running.base).await.expect("secret");

        let mut first = Browser::new();
        first.set_cookie("pvw_remember", &secret);
        assert_eq!(
            first.get(&running.base, "/api/account/remembered").await.1["restored"],
            json!(true)
        );
        let mut second = Browser::new();
        second.set_cookie("pvw_remember", &secret);
        assert_eq!(
            second.get(&running.base, "/api/account/remembered").await.1["restored"],
            json!(true)
        );
        assert_eq!(first.get(&running.base, "/api/devices").await.0, 200);
        assert_eq!(second.get(&running.base, "/api/devices").await.0, 200);

        assert_eq!(
            second
                .post(&running.base, "/api/account/signout", json!({}))
                .await
                .0,
            200
        );
        assert_eq!(first.get(&running.base, "/api/devices").await.0, 401);
        assert_eq!(owner.get(&running.base, "/api/devices").await.0, 401);

        let mut late = Browser::new();
        late.set_cookie("pvw_remember", &secret);
        let (_, reply) = late.get(&running.base, "/api/account/remembered").await;
        assert_eq!(reply["restored"], json!(false));
        running.shutdown.notify();
    });
}

#[test]
fn sign_out_without_a_credential_leaves_the_stored_token_alone() {
    block_on(async {
        let dir = data_dir("crosstest");
        let cloud = cloud_stub();
        let running = start(&dir, &cloud).await;
        let mut owner = Browser::new();
        assert_eq!(sign_in(&mut owner, &running.base).await, 200);
        let secret = remember(&mut owner, &running.base).await.expect("secret");

        let mut stranger = Browser::new();
        let (status, reply) = stranger
            .post(&running.base, "/api/account/signout", json!({}))
            .await;
        assert_eq!(status, 200);
        assert_eq!(reply["ok"], json!(true));
        assert_eq!(reply["email"], json!(""));

        let mut restoring = Browser::new();
        restoring.set_cookie("pvw_remember", &secret);
        assert_eq!(
            restoring
                .get(&running.base, "/api/account/remembered")
                .await
                .1["restored"],
            json!(true)
        );
        assert_eq!(restoring.get(&running.base, "/api/devices").await.0, 200);
        running.shutdown.notify();
    });
}

#[test]
fn the_routes_validate_the_serial_and_the_payload_before_calling_the_cloud() {
    block_on(async {
        let dir = data_dir("validate");
        let cloud = cloud_stub();
        let running = start(&dir, &cloud).await;

        let mut anonymous = Browser::new();
        let (status, _) = anonymous
            .post(
                &running.base,
                "/api/devices/bind",
                json!({ "sn": "AASN000000000002" }),
            )
            .await;
        assert_eq!(status, 401);

        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);

        let (status, reply) = browser
            .post(
                &running.base,
                "/api/devices/bind",
                json!({ "sn": "no", "username": "admin", "password": "x" }),
            )
            .await;
        assert_eq!(status, 400);
        assert_eq!(reply["code"], json!("bad_serial"));

        let (status, reply) = browser
            .post(&running.base, "/api/devices/unbind", json!({ "sn": "" }))
            .await;
        assert_eq!(status, 400);
        assert_eq!(reply["code"], json!("bad_serial"));

        let (status, reply) = browser
            .post(
                &running.base,
                "/api/devices/bind/qr",
                json!({ "payload": "see https://x.test/a b-z" }),
            )
            .await;
        assert_eq!(status, 400);
        assert_eq!(reply["code"], json!("bad_qr"));
        running.shutdown.notify();
    });
}

#[test]
fn the_bind_routes_report_the_vendor_codes_they_received() {
    block_on(async {
        let dir = data_dir("codes");
        let cloud = cloud_stub();
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);

        let (status, reply) = browser
            .post(
                &running.base,
                "/api/devices/bind",
                json!({ "sn": "AASN000000000001", "username": "admin", "password": "x" }),
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(reply["status"], json!("already_bound"));
        assert_eq!(reply["vendor_code"], json!(4101));
        assert!(reply["message"]
            .as_str()
            .unwrap()
            .contains("the camera password was not checked"));

        let (status, reply) = browser
            .post(
                &running.base,
                "/api/devices/bind",
                json!({ "sn": "AASN000000000002", "username": "admin", "password": "x" }),
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(reply["status"], json!("bound"));

        let (status, reply) = browser
            .post(
                &running.base,
                "/api/devices/bind",
                json!({ "sn": "AASN000000000003", "username": "admin", "password": "x" }),
            )
            .await;
        assert_eq!(status, 400);
        assert_eq!(reply["code"], json!("bind_rejected"));
        assert!(reply["message"]
            .as_str()
            .unwrap()
            .contains("DEV_USERNAME_FAIL"));

        let (status, _) = browser
            .post(
                &running.base,
                "/api/devices/bind",
                json!({ "sn": "AASN000000000004", "username": "admin", "password": "x" }),
            )
            .await;
        assert_eq!(status, 401);

        let (status, reply) = browser
            .post(
                &running.base,
                "/api/devices/unbind",
                json!({ "sn": "AASN000000000002" }),
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(reply["status"], json!("removed"));
        running.shutdown.notify();
    });
}
