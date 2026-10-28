use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};

use proview_core::cloud::{body_key, open, seal};
use proview_core::config::{Config, ProviderKind};
use proview_server::{serve_with_state, ServeOptions};
use reqwest::{Client, Method, Response};
use serde_json::{json, Value};

const UUID: &str = "000000000000000000000001";
const APP_KEY: &str = "test-app-key";
const APP_SECRET: &str = "0123456789abcdef0123456789abcdef";
const ACCOUNT: &str = "acct-0000000000000000000001";
const SERIAL: &str = "SN-0001";
const WANTED: &str = "zhexample@gmail.com";

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

type Calls = Arc<Mutex<Vec<(String, String)>>>;

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

fn cloud_stub(handler: impl Fn(&str, &str) -> Value + Send + 'static) -> (String, Calls) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("stub listener");
    let address = listener.local_addr().expect("stub address");
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let recorder = calls.clone();
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
            recorder
                .lock()
                .expect("recorded calls")
                .push((method.clone(), form.clone()));
            let payload = handler(&method, &form).to_string();
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
    (format!("http://{address}"), calls)
}

fn login_reply() -> Value {
    json!({
        "code": 2000,
        "msg": "SUCCESS",
        "data": { "accessToken": token(3600), "refreshToken": "refresh" }
    })
}

fn devices(names: &[&str]) -> Value {
    let mine: Vec<Value> = names
        .iter()
        .map(|uuid| json!({ "uuid": uuid, "nickname": "Gate" }))
        .collect();
    json!({ "code": 2000, "msg": "SUCCESS", "data": { "mine": mine, "share": [] } })
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
                self.cookies
                    .insert(name.trim().to_string(), value.trim().to_string());
            }
        }
        let text = response.text().await.unwrap_or_default();
        let parsed = serde_json::from_str(&text).unwrap_or(Value::String(text));
        (status, parsed)
    }

    async fn post(&mut self, base: &str, path: &str, body: Value) -> (u16, Value) {
        self.send(Method::POST, &format!("{base}{path}"), Some(body))
            .await
    }
}

fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pvw-share-{name}-{}", std::process::id()));
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
        share_base: cloud.to_string(),
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

fn searched(calls: &Calls) -> Vec<String> {
    calls
        .lock()
        .expect("calls")
        .iter()
        .filter(|(method, _)| method == "usersearch")
        .map(|(_, form)| form.clone())
        .collect()
}

fn invited(calls: &Calls) -> Vec<String> {
    calls
        .lock()
        .expect("calls")
        .iter()
        .filter(|(method, _)| method == "devicesTransOut")
        .map(|(_, form)| form.clone())
        .collect()
}

#[test]
fn a_hit_that_is_not_the_requested_account_is_a_404_and_sends_no_invitation() {
    block_on(async {
        let (cloud, calls) = cloud_stub(|method, _| match method {
            "loginExist" => login_reply(),
            "usersearch" => json!({ "code": 2000, "msg": "SUCCESS", "data": [
                { "account": "someone@else.com", "id": "stranger-id" }
            ]}),
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let dir = data_dir("stranger");
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/share",
                json!({ "uuids": [SERIAL], "account": WANTED }),
            )
            .await;
        assert_eq!(status, 404, "{body}");
        assert_eq!(searched(&calls).len(), 1);
        assert!(invited(&calls).is_empty());
        running.shutdown.notify();
    });
}

#[test]
fn a_masked_hit_beside_a_stranger_is_the_target_of_the_invitation() {
    block_on(async {
        let (cloud, calls) = cloud_stub(|method, _| match method {
            "loginExist" => login_reply(),
            "usersearch" => json!({ "code": 2000, "msg": "SUCCESS", "data": [
                { "account": "someone@else.com", "id": "stranger-id" },
                { "account": "zh***@gmail.com", "id": "wanted-id" }
            ]}),
            "devicesTransOut" => {
                json!({ "code": 2000, "msg": "SUCCESS", "data": { "transKey": "k1", "expireMillis": 1 } })
            }
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let dir = data_dir("masked");
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/share",
                json!({ "uuids": [SERIAL], "account": WANTED }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["data"]["transKey"], "k1");
        assert_eq!(body["resolved"], "zh***@gmail.com");
        let sent = invited(&calls);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("transInUserId=wanted-id"), "{}", sent[0]);
        assert!(!sent[0].contains("stranger-id"), "{}", sent[0]);
        running.shutdown.notify();
    });
}

#[test]
fn two_matching_hits_are_a_409_instead_of_a_guess() {
    block_on(async {
        let (cloud, calls) = cloud_stub(|method, _| match method {
            "loginExist" => login_reply(),
            "usersearch" => json!({ "code": 2000, "msg": "SUCCESS", "data": [
                { "account": "zh***@gmail.com", "id": "one" },
                { "account": "zhe***@gmail.com", "id": "two" }
            ]}),
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let dir = data_dir("ambiguous");
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/share",
                json!({ "uuids": [SERIAL], "account": WANTED }),
            )
            .await;
        assert_eq!(status, 409, "{body}");
        assert!(invited(&calls).is_empty());
        running.shutdown.notify();
    });
}

#[test]
fn an_explicit_account_id_skips_the_search() {
    block_on(async {
        let (cloud, calls) = cloud_stub(|method, _| match method {
            "loginExist" => login_reply(),
            "devicesTransOut" => {
                json!({ "code": 2000, "msg": "SUCCESS", "data": { "transKey": "k2" } })
            }
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let dir = data_dir("explicit");
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/share",
                json!({ "uuids": [SERIAL], "acceptId": "given-id" }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert!(searched(&calls).is_empty());
        let sent = invited(&calls);
        assert!(sent[0].contains("transInUserId=given-id"), "{}", sent[0]);
        assert!(body.get("resolved").is_none());
        running.shutdown.notify();
    });
}

#[test]
fn an_acceptance_the_vendor_acknowledges_but_never_lands_is_a_409() {
    block_on(async {
        let (cloud, _) = cloud_stub(|method, _| match method {
            "loginExist" => login_reply(),
            "mdshareaccept" => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
            "msdlistCode" => devices(&["SOME-OTHER-CAMERA"]),
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let dir = data_dir("no-landing");
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/share/accept",
                json!({ "devId": SERIAL, "password": "secret" }),
            )
            .await;
        assert_eq!(status, 409, "{body}");
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("did not take effect"),
            "{body}"
        );
        running.shutdown.notify();
    });
}

#[test]
fn an_acceptance_that_lands_in_the_device_list_is_a_200() {
    block_on(async {
        let (cloud, _) = cloud_stub(|method, _| match method {
            "loginExist" => login_reply(),
            "mdshareaccept" => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
            "msdlistCode" => devices(&[SERIAL]),
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let dir = data_dir("landing");
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/share/accept",
                json!({ "devId": SERIAL, "password": "secret" }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["materialised"], true);
        running.shutdown.notify();
    });
}
