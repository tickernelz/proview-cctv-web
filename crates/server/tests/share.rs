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
const WANTED: &str = "xample@example.invalid";

static ENV: Once = Once::new();
static LOGS: Once = Once::new();
static LOGGED: Mutex<Vec<u8>> = Mutex::new(Vec::new());

struct Captured;

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        LOGGED.lock().expect("logs").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn capture_logs() {
    LOGS.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(|| Captured)
            .try_init();
    });
}

fn prepare_env() {
    ENV.call_once(|| {
        std::env::set_var("PROVIEW_APP_UUID", UUID);
        std::env::set_var("PROVIEW_APP_KEY", APP_KEY);
        std::env::set_var("PROVIEW_APP_SECRET", APP_SECRET);
        std::env::set_var("PROVIEW_DEVICE_SERIAL", CONFIGURED);
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
            let (route, query) = path.split_once('?').unwrap_or((path.as_str(), ""));
            let version = route.split('/').nth(2).unwrap_or_default().to_string();
            let tsid = route.split('/').nth(3).unwrap_or_default().to_string();
            let length = content_length(&head_text);
            let body = if buffer.len() >= head + 4 {
                String::from_utf8_lossy(&buffer[head + 4..(head + 4 + length).min(buffer.len())])
                    .trim()
                    .to_string()
            } else {
                String::new()
            };
            let key = body_key(&tsid, APP_SECRET);
            let form = if version == "v1" {
                query.to_string()
            } else if body.is_empty() {
                String::new()
            } else {
                open(&key, &body).unwrap_or_default()
            };
            {
                let mut recorded = recorder.lock().expect("recorded calls");
                recorded.push((method.clone(), form.clone()));
                recorded.push((
                    format!("{method}@wire"),
                    format!("version={version}&body={}", body.len()),
                ));
            }
            let reply = handler(&method, &form);
            if reply.is_null() {
                continue;
            }
            let payload = reply.to_string();
            let sealed = if version == "v1" {
                form_encoded(&payload)
            } else {
                seal(&key, &payload).unwrap_or(payload)
            };
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

fn form_encoded(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'*') {
            out.push(byte as char);
        } else if byte == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
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
                { "account": "x***@example.invalid", "id": "wanted-id" }
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
        assert_eq!(body["resolved"], "x***@example.invalid");
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
                { "account": "x***@example.invalid", "id": "one" },
                { "account": "xa***@example.invalid", "id": "two" }
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

fn forms(calls: &Calls, endpoint: &str) -> Vec<String> {
    calls
        .lock()
        .expect("calls")
        .iter()
        .filter(|(method, _)| method == endpoint)
        .map(|(_, form)| form.clone())
        .collect()
}

fn keys(form: &str) -> Vec<&str> {
    form.split('&')
        .filter_map(|pair| pair.split_once('=').map(|(key, _)| key))
        .collect()
}

fn field(form: &str, name: &str) -> String {
    form.split('&')
        .find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then(|| percent(value))
        })
        .unwrap_or_default()
}

fn percent(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(&text[index + 1..index + 3], 16) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(if bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

const SHAREABLE: &str = "SN00000001";
const CONFIGURED: &str = "SNCONFIGURED01";
const DEVICE_PASSWORD: &str = "device-password";

fn powers_plain(form: &str) -> String {
    let powers: Value = serde_json::from_str(&field(form, "powers")).expect("powers json");
    let blob = powers["devInfo"].as_str().expect("devInfo");
    proview_core::cloud::bind::decode_qr_blob(blob).expect("decodes")
}

async fn permitted_share(uuid: &str, tag: &str) -> (Calls, Value) {
    let (cloud, calls) = cloud_stub(|method, _| match method {
        "loginExist" => login_reply(),
        _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
    });
    let running = start(&data_dir(tag), &cloud).await;
    let mut browser = Browser::new();
    assert_eq!(sign_in(&mut browser, &running.base).await, 200);
    let (status, body) = browser
        .post(
            &running.base,
            "/api/cloud/device/share/permitted",
            json!({ "uuid": uuid, "acceptId": "wanted-id", "permissions": ["DP_PTZ"] }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    running.shutdown.notify();
    (calls, body)
}

async fn qr_share(uuid: &str, tag: &str) -> String {
    let (cloud, calls) = cloud_stub(|method, _| match method {
        "loginExist" => login_reply(),
        "appInfoSave" => {
            json!({ "code": 2000, "msg": "SUCCESS", "data": { "key": "key-1", "expireTime": 1 } })
        }
        _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
    });
    let running = start(&data_dir(tag), &cloud).await;
    let mut browser = Browser::new();
    assert_eq!(sign_in(&mut browser, &running.base).await, 200);
    let (status, body) = browser
        .post(
            &running.base,
            "/api/cloud/device/share/qr",
            json!({ "uuid": uuid, "permissions": ["DP_PTZ"] }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    running.shutdown.notify();
    let saved = forms(&calls, "appInfoSave");
    assert_eq!(saved.len(), 1);
    assert!(!saved[0].contains(DEVICE_PASSWORD), "{}", saved[0]);
    proview_core::cloud::bind::decode_qr_blob(&field(&saved[0], "info")).expect("blob")
}

#[test]
fn another_cameras_share_never_carries_the_configured_camera_password() {
    block_on(async {
        let (calls, _) = permitted_share(SHAREABLE, "other-cam").await;
        let sent = forms(&calls, "mdshareadd");
        assert_eq!(sent.len(), 1);
        assert!(!sent[0].contains(DEVICE_PASSWORD), "{}", sent[0]);
        let plain = powers_plain(&sent[0]);
        assert!(!plain.contains(DEVICE_PASSWORD), "{plain}");
        let parts: Vec<&str> = plain.split(',').collect();
        assert_eq!(parts[0], SHAREABLE);
        assert_eq!(parts[2], "");
        let qr = qr_share(SHAREABLE, "other-qr").await;
        assert!(!qr.contains(DEVICE_PASSWORD), "{qr}");
        let info: Value = serde_json::from_str(&qr).expect("info json");
        assert_eq!(info["pwd"], "");
        assert_eq!(info["devId"], SHAREABLE);
    });
}

#[test]
fn the_configured_camera_shares_its_own_password_whatever_the_letter_case() {
    block_on(async {
        let lower = CONFIGURED.to_lowercase();
        let (calls, _) = permitted_share(&lower, "own-cam").await;
        let plain = powers_plain(&forms(&calls, "mdshareadd")[0]);
        assert_eq!(plain.split(',').nth(2), Some(DEVICE_PASSWORD), "{plain}");
    });
}

#[test]
fn a_permission_share_sends_the_device_the_target_the_powers_and_the_chosen_rights() {
    block_on(async {
        let (cloud, calls) = cloud_stub(|method, _| match method {
            "loginExist" => login_reply(),
            "usersearch" => json!({ "code": 2000, "msg": "SUCCESS", "data": [
                { "account": "x***@example.invalid", "id": "wanted-id" }
            ]}),
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let dir = data_dir("permitted");
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/share/permitted",
                json!({ "uuid": SHAREABLE, "account": WANTED, "permissions": ["DP_Intercom", "DP_PTZ"] }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["permissions"], "DP_PTZ,DP_Intercom");
        let sent = forms(&calls, "mdshareadd");
        assert_eq!(sent.len(), 1);
        let form = &sent[0];
        assert_eq!(
            keys(form),
            vec!["shareUuid", "acceptId", "powers", "permissions"]
        );
        assert_eq!(
            forms(&calls, "mdshareadd@wire"),
            vec!["version=v1&body=0".to_string()]
        );
        assert_eq!(field(form, "shareUuid"), SHAREABLE);
        assert_eq!(field(form, "acceptId"), "wanted-id");
        assert_eq!(field(form, "permissions"), "DP_PTZ,DP_Intercom");
        let powers: Value = serde_json::from_str(&field(form, "powers")).expect("powers json");
        let blob = powers["devInfo"].as_str().expect("devInfo");
        let plain = proview_core::cloud::bind::decode_qr_blob(blob).expect("decodes");
        let parts: Vec<&str> = plain.split(',').collect();
        assert_eq!(parts[0], SHAREABLE);
        assert_eq!(parts[1], "", "the native encoder blanks the admin user");
        assert_eq!(
            parts[2], "",
            "another camera never gets the configured password"
        );
        assert_eq!(parts[3], "0");
        assert!(parts[4].parse::<i64>().expect("time") > 1_700_000_000);
        running.shutdown.notify();
    });
}

#[test]
fn an_account_password_travels_as_the_apps_uname_and_upass_ahead_of_the_share() {
    block_on(async {
        let (cloud, calls) = cloud_stub(|method, _| match method {
            "loginExist" => login_reply(),
            "usersearch" => json!({ "code": 2000, "msg": "SUCCESS", "data": [
                { "account": "x***@example.invalid", "id": "wanted-id" }
            ]}),
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let running = start(&data_dir("upass"), &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/share/permitted",
                json!({ "uuid": SHAREABLE, "acceptId": "wanted-id", "permissions": ["DP_PTZ"], "password": "acct pass+1" }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        let form = &forms(&calls, "mdshareadd")[0];
        assert_eq!(
            keys(form),
            vec![
                "uname",
                "upass",
                "shareUuid",
                "acceptId",
                "powers",
                "permissions"
            ]
        );
        assert_eq!(field(form, "uname"), "someone@example.invalid");
        assert_eq!(field(form, "upass"), "acct pass+1");
        running.shutdown.notify();
    });
}

#[test]
fn a_failed_share_call_never_echoes_the_account_password() {
    capture_logs();
    block_on(async {
        let (cloud, calls) = cloud_stub(|method, _| match method {
            "loginExist" => login_reply(),
            "mdshareadd" => Value::Null,
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let running = start(&data_dir("leak"), &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/share/permitted",
                json!({ "uuid": SHAREABLE, "acceptId": "wanted-id", "permissions": ["DP_PTZ"], "password": "leak-check-pass-9" }),
            )
            .await;
        assert_eq!(status, 503, "{body}");
        assert_eq!(
            field(&forms(&calls, "mdshareadd")[0], "upass"),
            "leak-check-pass-9"
        );
        assert!(!body.to_string().contains("leak-check-pass"), "{body}");
        assert!(!body.to_string().contains("upass"), "{body}");
        let logs = String::from_utf8_lossy(&LOGGED.lock().expect("logs")).to_string();
        assert!(!logs.contains("leak-check-pass"), "{logs}");
        running.shutdown.notify();
    });
}

#[test]
fn an_unknown_permission_is_refused_before_any_vendor_call() {
    block_on(async {
        let (cloud, calls) = cloud_stub(|method, _| match method {
            "loginExist" => login_reply(),
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let dir = data_dir("badperm");
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/share/permitted",
                json!({ "uuid": SHAREABLE, "acceptId": "x", "permissions": ["DP_Everything"] }),
            )
            .await;
        assert_eq!(status, 400, "{body}");
        assert!(forms(&calls, "mdshareadd").is_empty());
        assert!(forms(&calls, "usersearch").is_empty());
        running.shutdown.notify();
    });
}

#[test]
fn the_owner_lists_changes_and_revokes_a_share_by_its_id() {
    block_on(async {
        let (cloud, calls) = cloud_stub(|method, _| match method {
            "loginExist" => login_reply(),
            "mdsharemylist" => json!({ "code": 2000, "msg": "SUCCESS", "data": [
                { "id": "share-7", "uuid": SHAREABLE, "shareTime": 1793352845004i64, "ret": 0, "pid": "",
                  "powers": "{\"devInfo\":\"z1evz\"}", "account": "x***@example.invalid", "shareAccountId": "wanted-id",
                  "permissions": [
                    { "label": "DP_AlarmPush", "enabled": false },
                    { "label": "DP_CLoudConfig", "enabled": false },
                    { "label": "DP_Intercom", "enabled": true },
                    { "label": "DP_LocalStorage", "enabled": false },
                    { "label": "DP_ModifyPwd", "enabled": false },
                    { "label": "DP_PTZ", "enabled": true },
                    { "label": "DP_VideoCall", "enabled": false }
                  ] }
            ]}),
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let dir = data_dir("manage");
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (status, rows) = browser
            .send(
                Method::GET,
                &format!("{}/api/cloud/device/shares?uuid={SHAREABLE}", running.base),
                None,
            )
            .await;
        assert_eq!(status, 200, "{rows}");
        assert_eq!(rows[0]["shareId"], "share-7");
        assert_eq!(rows[0]["permissions"], json!(["DP_Intercom", "DP_PTZ"]));
        assert_eq!(rows[0]["state"], "0");
        assert_eq!(rows[0]["userId"], "wanted-id");
        assert_eq!(
            forms(&calls, "mdsharemylist@wire"),
            vec!["version=v1&body=0".to_string()]
        );
        assert_eq!(
            field(&forms(&calls, "mdsharemylist")[0], "shareUuid"),
            SHAREABLE
        );
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/shares/permissions",
                json!({ "shareId": "share-7", "permissions": ["DP_PTZ"] }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        let changed = forms(&calls, "mdsharesetpermission");
        assert_eq!(field(&changed[0], "shareId"), "share-7");
        assert_eq!(field(&changed[0], "permissions"), "DP_PTZ");
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/shares/revoke",
                json!({ "shareId": "share-7" }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(forms(&calls, "mdsharedel")[0], "devId=share-7");
        assert_eq!(
            forms(&calls, "mdsharedel@wire"),
            vec!["version=v1&body=0".to_string()]
        );
        let (status, _) = browser
            .post(
                &running.base,
                "/api/cloud/device/shares/revoke",
                json!({ "shareId": "../x" }),
            )
            .await;
        assert_eq!(status, 400);
        assert_eq!(forms(&calls, "mdsharedel").len(), 1);
        running.shutdown.notify();
    });
}

#[test]
fn a_vendor_refusal_of_a_permission_share_names_the_vendor_code() {
    block_on(async {
        let (cloud, _) = cloud_stub(|method, _| match method {
            "loginExist" => login_reply(),
            "mdshareadd" => json!({ "code": 4124, "msg": "DEV_SHARE_NOT_DEV", "data": "" }),
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let dir = data_dir("refused");
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/share/permitted",
                json!({ "uuid": SHAREABLE, "acceptId": "wanted-id", "permissions": ["DP_PTZ"] }),
            )
            .await;
        assert_eq!(status, 400, "{body}");
        assert!(body.to_string().contains("4124"), "{body}");
        running.shutdown.notify();
    });
}

#[test]
fn a_share_code_is_saved_for_thirty_minutes_and_comes_back_as_a_link_and_a_qr() {
    block_on(async {
        let (cloud, calls) = cloud_stub(|method, _| match method {
            "loginExist" => login_reply(),
            "appInfoSave" => {
                json!({ "code": 2000, "msg": "SUCCESS", "data": { "key": "key-1", "expireTime": 1793200000000i64 } })
            }
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let dir = data_dir("qr");
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (status, body) = browser
            .post(
                &running.base,
                "/api/cloud/device/share/qr",
                json!({ "uuid": SHAREABLE, "permissions": ["DP_PTZ"] }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["link"], "https://d.xmeye.net/CSee?shareKey=key-1");
        assert!(
            body["svg"].as_str().unwrap_or_default().contains("<svg"),
            "{body}"
        );
        assert_eq!(body["expiresAt"], 1793200000000i64);
        let saved = forms(&calls, "appInfoSave");
        assert_eq!(saved.len(), 1);
        assert_eq!(field(&saved[0], "expireTime"), "1800000");
        let plain =
            proview_core::cloud::bind::decode_qr_blob(&field(&saved[0], "info")).expect("blob");
        let info: Value = serde_json::from_str(&plain).expect("info json");
        assert_eq!(info["devId"], SHAREABLE);
        assert_eq!(info["userId"], ACCOUNT);
        assert_eq!(info["loginName"], "admin");
        assert_eq!(info["permissions"], "DP_PTZ");
        running.shutdown.notify();
    });
}

#[test]
fn a_scanned_share_key_is_resolved_through_the_cloud_before_binding() {
    block_on(async {
        let info = json!({ "devId": SHAREABLE, "devType": 0, "userId": "owner", "loginName": "admin", "pwd": "pw", "shareTimes": 1 }).to_string();
        let blob = proview_core::cloud::bind::encode_qr_blob(&info, 123);
        let (cloud, calls) = cloud_stub(move |method, _| match method {
            "loginExist" => login_reply(),
            "appInfoQueryLimit" => json!({ "code": 2000, "msg": "SUCCESS", "data": blob.clone() }),
            "mdadd" => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
            _ => json!({ "code": 2000, "msg": "SUCCESS", "data": "" }),
        });
        let dir = data_dir("scan");
        let running = start(&dir, &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base).await, 200);
        let (_, body) = browser
            .post(
                &running.base,
                "/api/devices/bind/qr",
                json!({ "payload": "https://d.xmeye.net/CSee?shareKey=key-1" }),
            )
            .await;
        let asked = forms(&calls, "appInfoQueryLimit");
        assert_eq!(asked.len(), 1, "{body}");
        assert_eq!(field(&asked[0], "key"), "key-1");
        let bound = calls
            .lock()
            .expect("calls")
            .iter()
            .filter(|(method, _)| method != "loginExist" && method != "appInfoQueryLimit")
            .map(|(_, form)| form.clone())
            .collect::<Vec<_>>();
        assert!(
            bound.iter().any(|form| form.contains(SHAREABLE)),
            "{bound:?} {body}"
        );
        running.shutdown.notify();
    });
}
