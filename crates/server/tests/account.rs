use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use proview_core::cloud::{body_key, open, seal};
use proview_core::config::{Config, ProviderKind};
use proview_server::{serve_with_state, ServeOptions};
use reqwest::{Client, Method, Response};
use serde_json::{json, Value};

const UUID: &str = "000000000000000000000001";
const APP_KEY: &str = "test-app-key";
const APP_SECRET: &str = "0123456789abcdef0123456789abcdef";
const ACCOUNT: &str = "acct-0000000000000000000001";
const EMAIL: &str = "someone@example.invalid";
const PASSWORD: &str = "old-secret";
const NEXT: &str = "new&secret=1";
const WINDOW_MS: u64 = 400;

static ENV: Once = Once::new();

fn prepare_env() {
    ENV.call_once(|| {
        std::env::set_var("PROVIEW_APP_UUID", UUID);
        std::env::set_var("PROVIEW_APP_KEY", APP_KEY);
        std::env::set_var("PROVIEW_APP_SECRET", APP_SECRET);
        std::env::set_var("PROVIEW_ACCOUNT_WINDOW_MS", WINDOW_MS.to_string());
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

fn token(serial: usize) -> String {
    let header = base64url(br#"{"typ":"JWT","alg":"HS256"}"#);
    let payload = base64url(
        format!(
            r#"{{"iss":"rs","appKey":"{APP_KEY}","account":"{ACCOUNT}","exp":{},"n":{serial}}}"#,
            now() + 3600
        )
        .as_bytes(),
    );
    format!("{header}.{payload}.signature")
}

#[derive(Clone, Debug)]
struct Call {
    method: String,
    form: String,
    auth: String,
}

type Calls = Arc<Mutex<Vec<Call>>>;

fn find_head(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn header(head: &str, wanted: &str) -> Option<String> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(wanted)
            .then(|| value.trim().to_string())
    })
}

fn serve(
    mut stream: TcpStream,
    handler: &(dyn Fn(&str, &str) -> Option<Value> + Send + Sync),
    calls: &Calls,
) {
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
            let length = header(&head_text, "content-length")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            if buffer.len() >= head + 4 + length {
                break;
            }
        }
    }
    let head = find_head(&buffer).unwrap_or(buffer.len());
    let head_text = String::from_utf8_lossy(&buffer[..head]).to_string();
    let path = head_text
        .lines()
        .next()
        .unwrap_or_default()
        .split(' ')
        .nth(1)
        .unwrap_or("/")
        .to_string();
    let method = path
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or_default()
        .to_string();
    let tsid = path.split('/').nth(3).unwrap_or_default().to_string();
    let length: usize = header(&head_text, "content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
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
    calls.lock().expect("calls").push(Call {
        method: method.clone(),
        form: form.clone(),
        auth: header(&head_text, "authorization").unwrap_or_default(),
    });
    let Some(reply) = handler(&method, &form) else {
        std::thread::sleep(Duration::from_secs(30));
        return;
    };
    let payload = reply.to_string();
    let sealed = seal(&key, &payload).unwrap_or(payload);
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        sealed.len(),
        sealed
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn cloud_stub(
    handler: impl Fn(&str, &str) -> Option<Value> + Send + Sync + 'static,
) -> (String, Calls) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("stub listener");
    let address = listener.local_addr().expect("stub address");
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let recorder = calls.clone();
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handler = handler.clone();
            let recorder = recorder.clone();
            std::thread::spawn(move || serve(stream, handler.as_ref(), &recorder));
        }
    });
    (format!("http://{address}"), calls)
}

fn ok(data: Value) -> Option<Value> {
    Some(json!({ "code": 2000, "msg": "SUCCESS", "data": data }))
}

fn logins(password: &'static str) -> impl Fn(&str) -> Option<Value> + Send + Sync {
    let issued = AtomicUsize::new(0);
    move |form: &str| {
        let wanted = format!("pass={}", encode(password));
        if form.ends_with(&wanted) {
            let serial = issued.fetch_add(1, Ordering::SeqCst);
            ok(json!({ "accessToken": token(serial), "refreshToken": "refresh" }))
        } else {
            Some(json!({ "code": 4000, "msg": "PASSWORD_ERROR", "data": "" }))
        }
    }
}

fn encode(value: &str) -> String {
    proview_core::cloud::url_encode(value)
}

fn of(calls: &Calls, method: &str) -> Vec<Call> {
    calls
        .lock()
        .expect("calls")
        .iter()
        .filter(|call| call.method == method)
        .cloned()
        .collect()
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
        let response: Response = tokio::time::timeout(Duration::from_secs(4), request.send())
            .await
            .expect("the server answered inside 4 s")
            .expect("request");
        let status = response.status().as_u16();
        for value in response.headers().get_all("set-cookie") {
            let text = value.to_str().unwrap_or_default().to_string();
            let first = text.split(';').next().unwrap_or_default();
            if let Some((name, value)) = first.split_once('=') {
                if value.is_empty() {
                    self.cookies.remove(name.trim());
                } else {
                    self.cookies
                        .insert(name.trim().to_string(), value.trim().to_string());
                }
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

    async fn get(&mut self, base: &str, path: &str) -> (u16, Value) {
        self.send(Method::GET, &format!("{base}{path}"), None).await
    }
}

fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pvw-account-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("data dir");
    dir
}

struct Running {
    base: String,
    dir: PathBuf,
    shutdown: rocket::Shutdown,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.shutdown.clone().notify();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn start(name: &str, cloud: &str) -> Running {
    start_with(name, cloud, ProviderKind::Proview, 3600).await
}

async fn start_with(
    name: &str,
    cloud: &str,
    provider: ProviderKind,
    session_ttl_secs: u64,
) -> Running {
    prepare_env();
    let dir = data_dir(name);
    let config = Config {
        provider,
        cloud_base: cloud.to_string(),
        share_base: cloud.to_string(),
        device_user: "admin".to_string(),
        device_pass: "device-password".to_string(),
        fixture_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures").to_string(),
        device_host: None,
        device_onvif: None,
        device_rtsp: None,
        session_ttl_secs,
    };
    let settings = proview_server::settings::SettingsStore::load(Some(&dir));
    let opts = ServeOptions {
        address: "127.0.0.1".to_string(),
        port: 0,
        secret_key: Some("uVo3haa/znSxx4MPMV95m4oe9EYQe99QbYgEuBtVqqU=".to_string()),
        data_dir: Some(dir.clone()),
    };
    let handle = serve_with_state(opts, config, settings)
        .await
        .expect("server");
    let base = format!("http://{}", handle.address);
    wait_ready(&base).await;
    Running {
        base,
        dir,
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
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the server did not answer on {base}");
}

async fn sign_in(browser: &mut Browser, base: &str, password: &str) -> u16 {
    browser
        .post(
            base,
            "/api/login",
            json!({ "email": EMAIL, "password": password }),
        )
        .await
        .0
}

#[test]
fn the_profile_comes_from_the_normal_sign_in_token() {
    block_on(async {
        let login = logins(PASSWORD);
        let (cloud, calls) = cloud_stub(move |method, form| match method {
            "loginExist" => login(form),
            "userinfo2" => ok(json!({
                "id": "user-1",
                "userId": "user-1",
                "username": "someone",
                "mail": "someone@example.invalid",
                "phone": "",
                "nickname": "Some One",
                "registerTime": 1700000000,
                "passUpdateTime": 1700000100
            })),
            _ => ok(json!("")),
        });
        let running = start("profile", &cloud).await;
        let mut browser = Browser::new();
        let (status, body) = browser.get(&running.base, "/api/cloud/profile").await;
        assert_eq!(status, 401, "{body}");
        assert!(of(&calls, "userinfo2").is_empty());
        assert_eq!(sign_in(&mut browser, &running.base, PASSWORD).await, 200);
        let (status, body) = browser.get(&running.base, "/api/cloud/profile").await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["id"], "user-1");
        assert_eq!(body["username"], "someone");
        assert_eq!(body["mail"], "so***@example.invalid");
        assert_eq!(body["nickname"], "Some One");
        assert_eq!(body["registerTime"], "1700000000");
        assert_eq!(body["passUpdateTime"], "1700000100");
        assert_eq!(body["source"], "cloud");
        let info = of(&calls, "userinfo2");
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].auth, token(0));
        assert_eq!(info[0].form, "");
    });
}

#[test]
fn a_password_change_verifies_the_old_password_then_sends_new_and_confirm() {
    block_on(async {
        let login = logins(PASSWORD);
        let (cloud, calls) = cloud_stub(move |method, form| match method {
            "loginExist" if form.ends_with(&format!("pass={}", encode(NEXT))) => {
                ok(json!({ "accessToken": token(99), "refreshToken": "r" }))
            }
            "loginExist" => login(form),
            "usercp" => ok(json!("")),
            _ => ok(json!("")),
        });
        let running = start("password", &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base, PASSWORD).await, 200);

        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/password",
                json!({ "current": "wrong", "next": NEXT }),
            )
            .await;
        assert_eq!(status, 403, "{body}");
        assert!(
            of(&calls, "usercp").is_empty(),
            "a wrong old password must not change anything"
        );

        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/password",
                json!({ "current": PASSWORD, "next": NEXT }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        let logins = of(&calls, "loginExist");
        let check = &logins[logins.len() - 2];
        assert_eq!(
            check.form,
            format!("account={EMAIL}&pass={}", encode(PASSWORD))
        );
        let changed = of(&calls, "usercp");
        assert_eq!(changed.len(), 1);
        let encoded = encode(NEXT);
        assert_eq!(
            changed[0].form,
            format!("newPass={encoded}&confirmPass={encoded}")
        );
        assert!(!changed[0].form.contains(&encode(PASSWORD)));
        assert_eq!(
            changed[0].auth,
            token(1),
            "usercp must use the token from the fresh check"
        );
        assert_ne!(changed[0].auth, token(0));

        browser.get(&running.base, "/api/cloud/profile").await;
        let info = of(&calls, "userinfo2");
        assert_eq!(info.last().map(|call| call.auth.clone()), Some(token(99)));
    });
}

#[test]
fn forgot_password_sends_checks_and_resets_with_the_vendor_user_id() {
    block_on(async {
        let (cloud, calls) = cloud_stub(|method, form| match method {
            "scmail" => ok(json!({ "userId": "uid-42" })),
            "usercpc" if form.ends_with("verCode=123456") => ok(json!({ "data": "x" })),
            "usercpc" => Some(json!({ "code": 4010, "msg": "CODE_ERROR", "data": "" })),
            "fpmail" => ok(json!("")),
            _ => ok(json!("")),
        });
        let running = start("forgot", &cloud).await;
        let mut browser = Browser::new();
        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/forgot/reset",
                json!({ "email": EMAIL, "code": "123456", "password": "p" }),
            )
            .await;
        assert_eq!(status, 409, "{body}");

        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/forgot/code",
                json!({ "email": EMAIL }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert!(
            body.get("userId").is_none(),
            "the vendor user id stays on the server"
        );
        let sent = of(&calls, "scmail");
        assert_eq!(
            sent[0].form,
            "mail=someone%40example.invalid&type=fp&language=2&username=&codeSize=6"
        );
        assert_eq!(sent[0].auth, "");

        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/forgot/check",
                json!({ "email": EMAIL, "code": "000000" }),
            )
            .await;
        assert_eq!(status, 400, "{body}");
        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/forgot/reset",
                json!({ "email": EMAIL, "code": "000000", "password": "p" }),
            )
            .await;
        assert_eq!(status, 409, "{body}");

        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/forgot/check",
                json!({ "email": EMAIL, "code": "123456" }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(
            of(&calls, "usercpc").last().expect("check").form,
            "phone=&mail=someone%40example.invalid&verCode=123456"
        );

        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/forgot/reset",
                json!({ "email": EMAIL, "code": "123456", "password": "brand new" }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        let reset = of(&calls, "fpmail");
        assert_eq!(reset.len(), 1);
        assert_eq!(
            reset[0].form,
            "id=uid-42&mail=someone%40example.invalid&verCode=123456&newPass=brand%20new&confirmPass=brand%20new"
        );
    });
}

#[test]
fn register_checks_the_name_sends_a_code_and_registers() {
    block_on(async {
        let (cloud, calls) = cloud_stub(|method, form| match method {
            "usercheck" if form == "name=taken" => {
                Some(json!({ "code": 4012, "msg": "REG_USER_EXIST", "data": "" }))
            }
            "usercheck" => Some(json!({ "code": 4025, "msg": "REG_USER_NOT_EXIST", "data": "" })),
            "scmail" if form.starts_with("mail=taken%40") => {
                Some(json!({ "code": 4023, "msg": "REG_MAIL_EXIST", "data": "" }))
            }
            "scmail" => ok(json!("")),
            "userregMail" => ok(json!("")),
            _ => ok(json!("")),
        });
        let running = start("register", &cloud).await;
        let mut browser = Browser::new();
        let (status, body) = browser
            .get(&running.base, "/api/account/register/name?name=taken")
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["available"], false);
        let (status, body) = browser
            .get(&running.base, "/api/account/register/name?name=fresh")
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["available"], true);

        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/register/code",
                json!({ "email": "taken@example.invalid" }),
            )
            .await;
        assert_eq!(status, 400, "{body}");
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("already has a cloud account"),
            "{body}"
        );

        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/register/code",
                json!({ "email": EMAIL }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(
            of(&calls, "scmail")[1].form,
            "mail=someone%40example.invalid&type=re&language=2&username=&codeSize=6"
        );

        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/register",
                json!({ "username": "fresh", "email": EMAIL, "password": "pw+1", "code": "654321" }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(
            of(&calls, "userregMail")[0].form,
            "name=fresh&password=pw%2B1&confirmPass=pw%2B1&mail=someone%40example.invalid&verCode=654321"
        );
    });
}

#[test]
fn deleting_the_account_requests_a_code_then_needs_the_typed_account_and_confirm() {
    block_on(async {
        let login = logins(PASSWORD);
        let (cloud, calls) = cloud_stub(move |method, form| match method {
            "loginExist" => login(form),
            "userecancel" => ok(json!("")),
            _ => ok(json!("")),
        });
        let running = start("delete", &cloud).await;
        let mut browser = Browser::new();
        let (status, _) = browser
            .post(&running.base, "/api/account/delete/code", json!({}))
            .await;
        assert_eq!(status, 401);
        assert_eq!(sign_in(&mut browser, &running.base, PASSWORD).await, 200);

        let (status, body) = browser
            .post(&running.base, "/api/account/delete/code", json!({}))
            .await;
        assert_eq!(status, 200, "{body}");
        let first = of(&calls, "userecancel");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].form, "code=&language=2&codeSize=6");
        assert_eq!(first[0].auth, token(0));

        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/delete",
                json!({ "account": "other@example.invalid", "code": "112233", "confirm": true }),
            )
            .await;
        assert_eq!(status, 400, "{body}");
        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/delete",
                json!({ "account": EMAIL, "code": "112233", "confirm": false }),
            )
            .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(of(&calls, "userecancel").len(), 1);

        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/delete",
                json!({ "account": "SOMEONE@example.invalid", "code": "112233", "confirm": true }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        let both = of(&calls, "userecancel");
        assert_eq!(both.len(), 2);
        assert_eq!(both[1].form, "code=112233&language=2&codeSize=6");

        let (status, _) = browser.get(&running.base, "/api/me").await;
        assert_eq!(status, 401, "the deleted account is signed out");
    });
}

#[test]
fn a_silent_vendor_is_a_504_that_names_the_stage() {
    block_on(async {
        let login = logins(PASSWORD);
        let (cloud, _calls) = cloud_stub(move |method, form| match method {
            "loginExist" => login(form),
            "userinfo2" | "scmail" | "usercp" => None,
            _ => ok(json!("")),
        });
        let running = start("silent", &cloud).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base, PASSWORD).await, 200);

        let started = Instant::now();
        let (status, body) = browser.get(&running.base, "/api/cloud/profile").await;
        let elapsed = started.elapsed();
        assert_eq!(status, 504, "{body}");
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("userinfo2"),
            "{body}"
        );
        assert!(elapsed >= Duration::from_millis(WINDOW_MS), "{elapsed:?}");
        assert!(
            elapsed < Duration::from_millis(WINDOW_MS + 1500),
            "{elapsed:?}"
        );

        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/forgot/code",
                json!({ "email": EMAIL }),
            )
            .await;
        assert_eq!(status, 504, "{body}");
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("scmail"),
            "{body}"
        );

        let (status, body) = browser
            .post(
                &running.base,
                "/api/account/password",
                json!({ "current": PASSWORD, "next": NEXT }),
            )
            .await;
        assert_eq!(status, 504, "{body}");
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("usercp"),
            "{body}"
        );
    });
}

#[test]
fn a_session_without_a_cloud_token_gets_409_for_the_profile() {
    block_on(async {
        let (cloud, calls) = cloud_stub(|_, _| ok(json!("")));
        let running = start_with("profile-tokenless", &cloud, ProviderKind::Fixture, 3600).await;
        let mut browser = Browser::new();
        let (status, body) = browser
            .post(
                &running.base,
                "/api/login",
                json!({ "email": "fixture@example.invalid", "password": "fixture-password" }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        let (status, body) = browser.get(&running.base, "/api/cloud/profile").await;
        assert_eq!(status, 409, "{body}");
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("no cloud sign-in"),
            "{body}"
        );
        assert!(of(&calls, "userinfo2").is_empty());
    });
}

#[test]
fn an_expired_session_gets_401_for_the_profile() {
    block_on(async {
        let login = logins(PASSWORD);
        let (cloud, calls) = cloud_stub(move |method, form| match method {
            "loginExist" => login(form),
            _ => ok(json!("")),
        });
        let running = start_with("profile-expired", &cloud, ProviderKind::Proview, 1).await;
        let mut browser = Browser::new();
        assert_eq!(sign_in(&mut browser, &running.base, PASSWORD).await, 200);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let (status, body) = browser.get(&running.base, "/api/cloud/profile").await;
        assert_eq!(status, 401, "{body}");
        assert!(of(&calls, "userinfo2").is_empty());
    });
}
