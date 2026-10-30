use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

use proview_core::cloud::{body_key, open, seal};
use proview_core::config::{Config, ProviderKind};
use proview_server::{serve_with_state, ServeOptions};
use reqwest::Client;
use serde_json::{json, Value};

const UUID: &str = "000000000000000000000001";
const APP_KEY: &str = "test-app-key";
const APP_SECRET: &str = "0123456789abcdef0123456789abcdef";
const ACCOUNT: &str = "acct-0000000000000000000001";
const NO_PLAN: &str = "SN-0001";
const CAPS_DOWN: &str = "SN-0002";
const PLANNED: &str = "SN-0003";
const BUCKET: &str = "OSS_vid-01";
const INDEX: &str = "SN-0003/20261029/index.m3u8";
const WINDOW_MS: u64 = 800;

struct Request {
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> String {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

enum Reply {
    Answer(u16, &'static str, Vec<u8>),
    Hold,
}

fn json_reply(value: Value) -> Reply {
    Reply::Answer(200, "application/json", value.to_string().into_bytes())
}

fn find_head(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let head = loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(head) = find_head(&buffer) {
            break head;
        }
    };
    let head_text = String::from_utf8_lossy(&buffer[..head]).to_string();
    let mut lines = head_text.lines();
    let path = lines
        .next()
        .unwrap_or_default()
        .split(' ')
        .nth(1)
        .unwrap_or("/")
        .to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();
    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    while buffer.len() < head + 4 + length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let end = (head + 4 + length).min(buffer.len());
    Some(Request {
        path,
        headers,
        body: buffer[head + 4..end].to_vec(),
    })
}

fn serve(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake listener");
    let port = listener.local_addr().expect("fake address").port();
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let handler = handler.clone();
            std::thread::spawn(move || {
                let Some(request) = read_request(&mut stream) else {
                    return;
                };
                match handler(&request) {
                    Reply::Hold => std::thread::sleep(Duration::from_secs(6)),
                    Reply::Answer(status, kind, body) => {
                        let head = format!(
                            "HTTP/1.1 {status} X\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(&body);
                        let _ = stream.flush();
                    }
                }
            });
        }
    });
    port
}

fn segment(name: &str) -> Vec<u8> {
    let mut bytes = vec![0x47];
    bytes.extend_from_slice(name.as_bytes());
    bytes
}

fn caps_for(sn: &str) -> Value {
    let (enable, expires) = match sn {
        PLANNED => (true, 4_102_444_800i64),
        _ => (false, 0),
    };
    json!({
        "ret": 200, "msg": "Success", "sn": sn, "model": "fake", "pid": "fake",
        "caps": {
            "xmc.css.vid.support": true,
            "xmc.css.vid.enable": enable,
            "xmc.css.vid.expirationtime": expires,
            "xmc.css.vid.enable.channels": "",
            "xmc.css.vid.expirationtime.channels": ""
        }
    })
}

type Signed = Arc<Mutex<Vec<(String, Vec<String>)>>>;

fn signed_calls() -> &'static Signed {
    static CALLS: OnceLock<Signed> = OnceLock::new();
    CALLS.get_or_init(|| Arc::new(Mutex::new(Vec::new())))
}

fn css_fake(hls_port: u16) -> u16 {
    serve(move |request| {
        let body = request.json();
        let centre = &body["CssCenter"];
        let inner = &centre["Body"];
        let serial = inner["SerialNumber"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        match centre["Header"]["MessageType"].as_str().unwrap_or_default() {
            "MSG_GET_CSS_HLS_REQ" => {
                let (code, text) = if serial == PLANNED {
                    ("200", "Success OK")
                } else {
                    ("403", "Dev Not Open Cloud Storage")
                };
                json_reply(
                    json!({"CssCenter":{"Header":{"ErrorNum":code,"ErrorString":text,"MessageType":"MSG_GET_CSS_HLS_RSP","Version":"1.0"}}}),
                )
            }
            "MSG_DOWNLOAD_SIGN_REQ" => {
                let names: Vec<String> = inner["ObjInfo"]
                    .as_array()
                    .map(|entries| {
                        entries
                            .iter()
                            .map(|entry| entry["ObjName"].as_str().unwrap_or_default().to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                signed_calls().lock().expect("signed").push((
                    inner["ObjType"].as_str().unwrap_or_default().to_string(),
                    names.clone(),
                ));
                if names.iter().any(|name| name.contains("silentsign")) {
                    return Reply::Hold;
                }
                let info: Vec<Value> = names
                    .iter()
                    .map(|name| json!({
                        "Schema": "http",
                        "Host": format!("127.0.0.1:{hls_port}"),
                        "URL": format!("/{name}"),
                        "ReqHeader": { "Date": "Thu, 29 Oct 2026 08:00:00 GMT", "Authorization": format!("OSS fake:{name}") }
                    }))
                    .collect();
                json_reply(
                    json!({"CssCenter":{"Header":{"ErrorNum":"200","ErrorString":"Success OK","MessageType":"MSG_DOWNLOAD_SIGN_RSP"},"Body":{"Method":"GET","SerialNumber":serial,"ObjInfo":info}}}),
                )
            }
            _ => json_reply(
                json!({"CssCenter":{"Header":{"ErrorNum":"400","ErrorString":"invalid protocol format args"}}}),
            ),
        }
    })
}

fn hls_fake() -> u16 {
    serve(|request| {
        let name = request.path.trim_start_matches('/').to_string();
        if request.header("authorization") != format!("OSS fake:{name}") {
            return Reply::Answer(
                403,
                "application/xml",
                b"<Error>SignatureDoesNotMatch</Error>".to_vec(),
            );
        }
        let playlist = |lines: &str| {
            Reply::Answer(
                200,
                "application/vnd.apple.mpegurl",
                format!(
                    "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n{lines}#EXT-X-ENDLIST\n"
                )
                .into_bytes(),
            )
        };
        match name.as_str() {
            INDEX => playlist("#EXTINF:10.0,\nseg0.ts\n#EXTINF:4.0,\nseg1.ts\n"),
            "SN-0003/20261029/seg0.ts" => Reply::Answer(200, "video/mp2t", segment("seg-zero")),
            "SN-0003/20261029/seg1.ts" => Reply::Answer(200, "video/mp2t", segment("seg-one")),
            "SN-0003/slowseg/index.m3u8" => playlist("#EXTINF:10.0,\nstuck.ts\n"),
            "SN-0003/slowseg/stuck.ts" => Reply::Hold,
            _ => Reply::Answer(404, "application/xml", b"<Error>NoSuchKey</Error>".to_vec()),
        }
    })
}

fn pms_fake() -> u16 {
    serve(|request| {
        let body = request.json();
        if request.path != "/v2" || body["msg"] != "video_query" || body["auth"] != "pms-code" {
            return json_reply(json!({"code":"401","msg":"video_query"}));
        }
        if body["sn"] == PLANNED && body["st"] == "2026-10-29 00:00:00" {
            return json_reply(json!({"code":"200","msg":"video_query","vidlist":[
                {"indx": INDEX, "bucket": BUCKET, "label": "human", "st": "2026-10-29 08:00:00", "et": "2026-10-29 08:00:14"}
            ]}));
        }
        json_reply(json!({"code":"200","msg":"video_query"}))
    })
}

fn fake_ffmpeg() -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("pvw-clips-ffmpeg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("ffmpeg dir");
    let path = dir.join("ffmpeg");
    std::fs::write(&path, "#!/bin/sh\nprintf MP4:\ncat\n").expect("ffmpeg script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

static ENV: Once = Once::new();

fn prepare_env() {
    ENV.call_once(|| {
        let hls = hls_fake();
        let css = css_fake(hls);
        let caps = serve(|request| {
            let body = request.json();
            let sn = body["body"]["sn"].as_str().unwrap_or_default().to_string();
            if sn == CAPS_DOWN {
                return json_reply(json!({"ret": 500, "msg": "internal"}));
            }
            json_reply(caps_for(&sn))
        });
        let pms = pms_fake();
        std::env::set_var("PROVIEW_APP_UUID", UUID);
        std::env::set_var("PROVIEW_APP_KEY", APP_KEY);
        std::env::set_var("PROVIEW_APP_SECRET", APP_SECRET);
        std::env::set_var("PROVIEW_CAPS_BASE", format!("http://127.0.0.1:{caps}"));
        std::env::set_var("PROVIEW_CSS_HOST", format!("127.0.0.1:{css}"));
        std::env::set_var("PROVIEW_PMS_BASE", format!("http://127.0.0.1:{pms}"));
        std::env::set_var("PROVIEW_CLOUD_CLIP_WINDOW_MS", WINDOW_MS.to_string());
        std::env::set_var("PROVIEW_FFMPEG", fake_ffmpeg());
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

fn token() -> String {
    let header = base64url(br#"{"typ":"JWT","alg":"HS256"}"#);
    let payload = base64url(
        format!(
            r#"{{"iss":"rs","appKey":"{APP_KEY}","account":"{ACCOUNT}","exp":{}}}"#,
            now() + 3600
        )
        .as_bytes(),
    );
    format!("{header}.{payload}.signature")
}

fn envelope_fake() -> String {
    let port = serve(|request| {
        let mut parts = request.path.trim_start_matches('/').split('/');
        let method = parts.next().unwrap_or_default().to_string();
        let tsid = parts.nth(1).unwrap_or_default().to_string();
        let key = body_key(&tsid, APP_SECRET);
        let body = String::from_utf8_lossy(&request.body).trim().to_string();
        if !body.is_empty() {
            let _ = open(&key, &body);
        }
        let mine: Vec<Value> = [NO_PLAN, CAPS_DOWN, PLANNED]
            .iter()
            .map(|uuid| json!({ "uuid": uuid, "nickname": "Gate", "pms": "pms-code", "css": "css-code" }))
            .collect();
        let payload = match method.as_str() {
            "loginExist" => json!({"code": 2000, "msg": "SUCCESS", "data": {"accessToken": token(), "refreshToken": "refresh"}}),
            "msdlistCode" => json!({"code": 2000, "msg": "SUCCESS", "data": {"mine": mine, "share": []}}),
            _ => json!({"code": 2000, "msg": "SUCCESS", "data": ""}),
        }
        .to_string();
        let sealed = seal(&key, &payload).unwrap_or(payload);
        Reply::Answer(200, "text/html", sealed.into_bytes())
    });
    format!("http://127.0.0.1:{port}")
}

fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pvw-clips-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("data dir");
    dir
}

struct Running {
    base: String,
    cookie: String,
    http: Client,
    _shutdown: rocket::Shutdown,
}

struct Answer {
    status: u16,
    kind: String,
    disposition: String,
    bytes: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or(Value::Null)
    }
}

impl Running {
    async fn get(&self, path: &str) -> Answer {
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            self.http
                .get(format!("{}{path}", self.base))
                .header("cookie", &self.cookie)
                .send(),
        )
        .await
        .expect("the route answered inside 5 s")
        .expect("request");
        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string()
        };
        let kind = header("content-type");
        let disposition = header("content-disposition");
        let bytes = response.bytes().await.expect("body").to_vec();
        Answer {
            status,
            kind,
            disposition,
            bytes,
        }
    }
}

async fn start(dir: &Path) -> Running {
    prepare_env();
    let cloud = envelope_fake();
    let config = Config {
        provider: ProviderKind::Proview,
        cloud_base: cloud.clone(),
        share_base: cloud,
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
    let http = Client::new();
    for _ in 0..100 {
        if let Ok(response) = http.get(format!("{base}/api/health")).send().await {
            if response.status().is_success() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let login = http
        .post(format!("{base}/api/login"))
        .json(&json!({ "email": "someone@example.invalid", "password": "secret" }))
        .send()
        .await
        .expect("login");
    assert_eq!(login.status().as_u16(), 200, "fake sign-in");
    let cookie = login
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|text| text.split(';').next())
        .collect::<Vec<_>>()
        .join("; ");
    Running {
        base,
        cookie,
        http,
        _shutdown: handle.shutdown,
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

fn clip_query(device: &str, object: &str) -> String {
    format!(
        "device={device}&object={}&bucket={BUCKET}",
        object.replace('/', "%2F")
    )
}

fn expected_clip() -> Vec<u8> {
    let mut clip = segment("seg-zero");
    clip.extend(segment("seg-one"));
    clip
}

#[test]
fn the_list_reads_the_native_vidlist_from_the_v2_query() {
    block_on(async {
        let running = start(&data_dir("list")).await;
        let answer = running
            .get(&format!(
                "/api/recordings?device={PLANNED}&day=2026-10-29&channel=0"
            ))
            .await;
        assert_eq!(answer.status, 200, "{:?}", answer.json());
        let page = answer.json();
        assert_eq!(page["recordings"].as_array().map(Vec::len), Some(1));
        assert_eq!(page["recordings"][0]["index_file"], INDEX);
        assert_eq!(page["recordings"][0]["storage_bucket"], BUCKET);
        assert_eq!(page["recordings"][0]["duration_secs"], 14);
        let empty = running
            .get(&format!(
                "/api/recordings?device={NO_PLAN}&day=2026-10-29&channel=0"
            ))
            .await;
        assert_eq!(empty.status, 200);
        assert_eq!(empty.json()["recordings"].as_array().map(Vec::len), Some(0));
    });
}

#[test]
fn plan_state_reads_caps_like_the_app_and_maps_css_403_to_no_plan() {
    block_on(async {
        let running = start(&data_dir("state")).await;
        let none = running
            .get(&format!(
                "/api/cloud/clips/state?device={NO_PLAN}&day=2026-10-29"
            ))
            .await;
        assert_eq!(none.status, 200, "{:?}", none.json());
        let body = none.json();
        assert_eq!(body["plan"], "none");
        assert_eq!(body["source"], "caps");
        assert!(body["reason"]
            .as_str()
            .unwrap_or_default()
            .starts_with("No cloud storage plan on this camera"));
        assert_eq!(body["evidence"][1]["code"], "403");
        let css = running
            .get(&format!(
                "/api/cloud/clips/state?device={CAPS_DOWN}&day=2026-10-29"
            ))
            .await;
        let body = css.json();
        assert_eq!(css.status, 200, "{body:?}");
        assert_eq!(body["plan"], "none");
        assert_eq!(body["source"], "css");
        assert_eq!(body["code"], "403");
        assert_eq!(body["evidence"][1]["detail"], "Dev Not Open Cloud Storage");
        let open = running
            .get(&format!(
                "/api/cloud/clips/state?device={PLANNED}&day=2026-10-29"
            ))
            .await;
        assert_eq!(open.json()["plan"], "open");
        let stranger = running.get("/api/cloud/clips/state?device=SN-9999").await;
        assert_eq!(stranger.status, 404);
    });
}

#[test]
fn download_signs_the_index_then_the_segments_in_one_batch_and_joins_them() {
    block_on(async {
        let running = start(&data_dir("download")).await;
        let answer = running
            .get(&format!(
                "/api/cloud/clips/download?{}",
                clip_query(PLANNED, INDEX)
            ))
            .await;
        assert_eq!(
            answer.status,
            200,
            "{}",
            String::from_utf8_lossy(&answer.bytes)
        );
        assert_eq!(answer.kind, "video/mp2t");
        assert_eq!(
            answer.disposition,
            "attachment; filename=\"SN-0003-index.ts\""
        );
        assert_eq!(answer.bytes, expected_clip());
        let calls = signed_calls().lock().expect("signed").clone();
        let index_at = calls
            .iter()
            .position(|(kind, names)| kind == "VIDEO" && names == &vec![INDEX.to_string()])
            .expect("the index was signed as VIDEO");
        assert!(calls[index_at + 1..]
            .iter()
            .any(|(kind, names)| kind == "VIDEO"
                && names
                    == &vec![
                        "SN-0003/20261029/seg0.ts".to_string(),
                        "SN-0003/20261029/seg1.ts".to_string()
                    ]));
    });
}

#[test]
fn play_hands_the_joined_stream_to_the_remuxer_and_serves_mp4() {
    block_on(async {
        let running = start(&data_dir("play")).await;
        let answer = running
            .get(&format!(
                "/api/cloud/clips/play?{}",
                clip_query(PLANNED, INDEX)
            ))
            .await;
        assert_eq!(
            answer.status,
            200,
            "{}",
            String::from_utf8_lossy(&answer.bytes)
        );
        assert_eq!(answer.kind, "video/mp4");
        let mut expected = b"MP4:".to_vec();
        expected.extend(expected_clip());
        assert_eq!(answer.bytes, expected);
    });
}

#[test]
fn clips_of_another_device_or_without_a_bucket_are_refused_before_any_vendor_call() {
    block_on(async {
        let running = start(&data_dir("refuse")).await;
        let foreign = running
            .get(&format!(
                "/api/cloud/clips/download?{}",
                clip_query(PLANNED, "SN-0001/20261029/index.m3u8")
            ))
            .await;
        assert_eq!(foreign.status, 400);
        let traversal = running
            .get(&format!(
                "/api/cloud/clips/download?{}",
                clip_query(PLANNED, "SN-0003/../index.m3u8")
            ))
            .await;
        assert_eq!(traversal.status, 400);
        let bare = running
            .get(&format!(
                "/api/cloud/clips/download?device={PLANNED}&object=SN-0003%2Fa.m3u8"
            ))
            .await;
        assert_eq!(bare.status, 400);
        assert!(bare.json()["message"]
            .as_str()
            .unwrap_or_default()
            .contains("bucket"));
    });
}

#[test]
fn a_silent_sign_centre_is_a_504_naming_the_sign_stage() {
    block_on(async {
        let running = start(&data_dir("silentsign")).await;
        let started = Instant::now();
        let answer = running
            .get(&format!(
                "/api/cloud/clips/download?{}",
                clip_query(PLANNED, "SN-0003/silentsign/index.m3u8")
            ))
            .await;
        let elapsed = started.elapsed();
        assert_eq!(answer.status, 504);
        let message = answer.json()["message"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(message.contains("sign stage"), "{message}");
        assert!(elapsed >= Duration::from_millis(WINDOW_MS), "{elapsed:?}");
        assert!(
            elapsed < Duration::from_millis(WINDOW_MS + 1500),
            "{elapsed:?}"
        );
    });
}

#[test]
fn a_silent_segment_host_is_a_504_naming_the_segment_stage() {
    block_on(async {
        let running = start(&data_dir("silentseg")).await;
        let started = Instant::now();
        let answer = running
            .get(&format!(
                "/api/cloud/clips/play?{}",
                clip_query(PLANNED, "SN-0003/slowseg/index.m3u8")
            ))
            .await;
        let elapsed = started.elapsed();
        assert_eq!(answer.status, 504);
        let message = answer.json()["message"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(message.contains("segment stage"), "{message}");
        assert!(
            elapsed < Duration::from_millis(WINDOW_MS + 1500),
            "{elapsed:?}"
        );
    });
}
