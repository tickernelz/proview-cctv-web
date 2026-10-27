use std::time::Duration;

use serde_json::{json, Value};

use super::client::{CloudError, CloudResult};

pub const CSS_HOST: &str = "access-css.secu100.net";
pub const CSS_PORT: u16 = 6614;
pub const API_VERSION: &str = "1.0";
pub const SIGN_REQUEST: &str = "MSG_DOWNLOAD_SIGN_REQ";
pub const OBJECT_TYPE: &str = "PIC";
pub const CHANNEL: &str = "0";
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
pub const USER_AGENT: &str = "proview-cctv-web/0.1";
pub const STORAGE_HOSTS: [&str; 4] = [
    ".aliyuncs.com",
    ".myhuaweicloud.com",
    ".jftech.com",
    ".xmcsrv.net",
];

#[derive(Debug, Clone, Default)]
pub struct SignRequest {
    pub serial: String,
    pub auth_code: String,
    pub user_id: String,
    pub object: String,
    pub bucket: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignedFile {
    pub url: String,
    pub host: String,
    pub schema: String,
    pub method: String,
    pub date: String,
    pub authorization: String,
}

impl SignedFile {
    pub fn address(&self) -> String {
        format!(
            "{}://{}{}",
            self.schema.trim(),
            self.host.trim(),
            self.url.trim()
        )
    }
}

pub struct CssClient {
    endpoint: String,
    loopback: bool,
    http: reqwest::Client,
}

impl CssClient {
    pub fn new(host: impl AsRef<str>) -> Self {
        let host = host.as_ref().trim();
        let host = host
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .trim_end_matches('/');
        let (host, port) = match host
            .rsplit_once(':')
            .and_then(|(head, tail)| tail.parse::<u16>().ok().map(|port| (head, port)))
        {
            Some((head, port)) => (head, port),
            None => (host, CSS_PORT),
        };
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("css http client");
        Self {
            endpoint: format!("http://{host}:{port}/"),
            loopback: is_loopback(host),
            http,
        }
    }

    pub fn from_env() -> Self {
        let host = std::env::var("PROVIEW_CSS_HOST")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| CSS_HOST.to_string());
        Self::new(host)
    }

    pub fn endpoint(&self) -> String {
        self.endpoint.clone()
    }

    pub async fn sign(&self, request: &SignRequest) -> CloudResult<SignedFile> {
        let reply = self
            .http
            .post(&self.endpoint)
            .json(&request_body(request))
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.without_url().to_string()))?;
        let text = reply
            .text()
            .await
            .map_err(|error| CloudError::Unreachable(error.without_url().to_string()))?;
        parse_sign(&text)
    }

    pub async fn picture(&self, request: &SignRequest) -> CloudResult<Vec<u8>> {
        let signed = self.sign(request).await?;
        self.fetch(&signed).await
    }

    pub async fn fetch(&self, file: &SignedFile) -> CloudResult<Vec<u8>> {
        let url = target(file, self.loopback)?;
        let method = if file.method.trim().is_empty() {
            "GET"
        } else {
            file.method.trim()
        };
        let mut request = self.http.request(
            reqwest::Method::from_bytes(method.as_bytes()).map_err(|error| {
                CloudError::Rejected(format!("signed method {method}: {error}"))
            })?,
            url,
        );
        if !file.date.trim().is_empty() {
            request = request.header("Date", file.date.trim());
        }
        if !file.authorization.trim().is_empty() {
            request = request.header("Authorization", file.authorization.trim());
        }
        let reply = request
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.without_url().to_string()))?;
        let status = reply.status();
        let bytes = reply
            .bytes()
            .await
            .map_err(|error| CloudError::Unreachable(error.without_url().to_string()))?;
        if bytes.len() > 3 && bytes[0] == 0xff && bytes[1] == 0xd8 {
            return Ok(bytes.to_vec());
        }
        Err(CloudError::Malformed(format!(
            "object storage refused {status}: {}",
            brief(&bytes)
        )))
    }
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

pub fn target(file: &SignedFile, loopback: bool) -> CloudResult<reqwest::Url> {
    let address = file.address();
    let url = reqwest::Url::parse(&address)
        .map_err(|error| CloudError::Malformed(format!("signed file is not a url: {error}")))?;
    if loopback {
        return Ok(url);
    }
    if url.scheme() != "https" {
        return Err(CloudError::Malformed(format!(
            "signed file is not https: {}",
            url.scheme()
        )));
    }
    let host = url.host_str().unwrap_or_default();
    if !STORAGE_HOSTS
        .iter()
        .any(|suffix| host.ends_with(suffix) && host.len() > suffix.len())
    {
        return Err(CloudError::Malformed(format!(
            "signed file host is not object storage: {host}"
        )));
    }
    if !url.path().starts_with('/') || url.path().len() < 2 {
        return Err(CloudError::Malformed(
            "signed file path is not absolute".to_string(),
        ));
    }
    Ok(url)
}

fn brief(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let flat = text.split_whitespace().collect::<Vec<&str>>().join(" ");
    flat.chars().take(120).collect()
}

pub fn request_body(request: &SignRequest) -> Value {
    let mut body = json!({
        "SerialNumber": request.serial.trim(),
        "Encrypt": 1,
        "Channel": CHANNEL,
        "ObjType": OBJECT_TYPE,
        "ObjInfo": [{ "ObjName": request.object.trim(), "StorageBucket": request.bucket.trim() }],
    });
    if let Some(fields) = body.as_object_mut() {
        if !request.auth_code.trim().is_empty() {
            fields.insert("AuthCode".to_string(), json!(request.auth_code.trim()));
        }
        if !request.user_id.trim().is_empty() {
            fields.insert("UserId".to_string(), json!(request.user_id.trim()));
        }
    }
    json!({
        "CssCenter": {
            "Header": { "MessageType": SIGN_REQUEST, "Version": API_VERSION },
            "Body": body,
        }
    })
}

pub fn parse_sign(text: &str) -> CloudResult<SignedFile> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| CloudError::Rejected(format!("css sign reply is not json: {error}")))?;
    let centre = &value["CssCenter"];
    let header = &centre["Header"];
    let number = header["ErrorNum"].as_str().unwrap_or_default();
    let message = header["ErrorString"].as_str().unwrap_or_default();
    if number != "200" {
        return Err(CloudError::Rejected(format!(
            "css sign refused: {message} ({number})"
        )));
    }
    let body = &centre["Body"];
    let info = body["ObjInfo"]
        .as_array()
        .and_then(|entries| entries.first())
        .ok_or_else(|| CloudError::Rejected("css sign reply carries no object".to_string()))?;
    let method = body["Method"].as_str().unwrap_or("GET");
    Ok(SignedFile {
        url: text_of(info, "URL"),
        host: text_of(info, "Host"),
        schema: text_of(info, "Schema"),
        method: method.to_string(),
        date: text_of(&info["ReqHeader"], "Date"),
        authorization: text_of(&info["ReqHeader"], "Authorization"),
    })
}

fn text_of(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str = r#"{"CssCenter":{"Body":{"ObjInfo":[{"Schema":"https","URL":"/003_DEV0001_261027214632.jpeg","ReqHeader":{"Date":"Tue, 27 Oct 2026 16:09:33 GMT","Authorization":"OSS LTAI5tGmDfvPW9qvmCpKYPjp:mVf4taoOswlOV5yu7iENfQlmvjo="},"Host":"oss-ap-id-pic-01.oss-ap-southeast-5.aliyuncs.com"}],"SerialNumber":"DEV0001","Method":"GET"},"Header":{"Version":"1.0","MessageType":"MSG_DOWNLOAD_SIGN_RSP","ErrorNum":"200","ErrorString":"Success OK"}}}"#;
    const REFUSED: &str = r#"{"CssCenter":{"Header":{"CSeq":"1","ErrorNum":"401","ErrorString":"invalid sn","Version":"1.0","MessageType":"MSG_DOWNLOAD_SIGN_RSP"}}}"#;

    fn request() -> SignRequest {
        SignRequest {
            serial: "DEV0001".to_string(),
            auth_code: "aaaaaaaa-4a1c2f27d01793117365801".to_string(),
            user_id: "c6f8c33b29c95774c14bae9436e721c8".to_string(),
            object: "003_DEV0001_261027214632.jpeg".to_string(),
            bucket: "OSS_oss-ap-id-pic-01".to_string(),
        }
    }

    #[test]
    fn body_matches_the_sdk_shape() {
        let body = request_body(&request());
        let centre = &body["CssCenter"];
        assert_eq!(centre["Header"]["MessageType"], SIGN_REQUEST);
        assert_eq!(centre["Body"]["ObjType"], "PIC");
        assert!(centre["Body"]["ObjInfo"].is_array());
        assert_eq!(
            centre["Body"]["ObjInfo"][0]["StorageBucket"],
            "OSS_oss-ap-id-pic-01"
        );
        assert_eq!(
            centre["Body"]["AuthCode"],
            "aaaaaaaa-4a1c2f27d01793117365801"
        );
    }

    #[test]
    fn body_drops_empty_credentials() {
        let mut bare = request();
        bare.auth_code.clear();
        bare.user_id.clear();
        let body = request_body(&bare);
        assert!(body["CssCenter"]["Body"].get("AuthCode").is_none());
        assert!(body["CssCenter"]["Body"].get("UserId").is_none());
    }

    #[test]
    fn sign_reply_carries_the_storage_request() {
        let file = parse_sign(REPLY).expect("signed file");
        assert_eq!(
            file.address(),
            "https://oss-ap-id-pic-01.oss-ap-southeast-5.aliyuncs.com/003_DEV0001_261027214632.jpeg"
        );
        assert_eq!(file.method, "GET");
        assert_eq!(file.date, "Tue, 27 Oct 2026 16:09:33 GMT");
        assert!(file.authorization.starts_with("OSS "));
    }

    #[test]
    fn refusal_becomes_a_rejection() {
        let error = parse_sign(REFUSED).expect_err("refusal");
        assert!(error.to_string().contains("invalid sn"), "{error}");
    }

    #[test]
    fn picture_walks_the_sign_and_the_object() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind vendor");
        let port = listener.local_addr().expect("vendor address").port();
        let vendor = std::thread::spawn(move || {
            let (mut sign, _) = listener.accept().expect("sign accept");
            let mut buffer = [0u8; 8192];
            let read = sign.read(&mut buffer).expect("sign read");
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            assert!(request.starts_with("POST / HTTP/1.1"), "{request}");
            assert!(request.contains("MSG_DOWNLOAD_SIGN_REQ"), "{request}");
            assert!(
                request.contains("003_DEV0001_261027214632.jpeg"),
                "{request}"
            );
            let payload = REPLY
                .replace("\"Schema\":\"https\"", "\"Schema\":\"http\"")
                .replace(
                    "oss-ap-id-pic-01.oss-ap-southeast-5.aliyuncs.com",
                    &format!("127.0.0.1:{port}"),
                );
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            sign.write_all(reply.as_bytes()).expect("sign write");
            drop(sign);

            let (mut object, _) = listener.accept().expect("object accept");
            let read = object.read(&mut buffer).expect("object read");
            let request = String::from_utf8_lossy(&buffer[..read]).to_lowercase();
            assert!(
                request.starts_with("get /003_dev0001_261027214632.jpeg "),
                "{request}"
            );
            assert!(request.contains("authorization: oss "), "{request}");
            assert!(
                request.contains("date: tue, 27 oct 2026 16:09:33 gmt"),
                "{request}"
            );
            let body = [0xffu8, 0xd8, 0xff, 0xe0, 0x00, 0x10];
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            object.write_all(head.as_bytes()).expect("object head");
            object.write_all(&body).expect("object body");
        });

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let base = format!("127.0.0.1:{port}");
        let bytes = runtime
            .block_on(async { CssClient::new(&base).picture(&request()).await })
            .expect("picture");
        vendor.join().expect("vendor thread");
        assert_eq!(&bytes[..2], &[0xff, 0xd8]);
        assert_eq!(bytes.len(), 6);
    }

    #[test]
    #[ignore = "talks to the vendor cloud"]
    fn picture_comes_back_from_the_vendor() {
        let read = |key: &str| std::env::var(key).unwrap_or_default();
        let mut request = request();
        request.serial = read("PROVIEW_LIVE_SERIAL");
        request.auth_code = read("PROVIEW_LIVE_CODE");
        request.user_id = read("PROVIEW_LIVE_USER");
        request.bucket = read("PROVIEW_LIVE_BUCKET");
        request.object = read("PROVIEW_LIVE_OBJECT");
        assert!(!request.serial.is_empty(), "set PROVIEW_LIVE_SERIAL");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let client = CssClient::from_env();
        let mut sign_ms = Vec::new();
        let mut fetch_ms = Vec::new();
        let mut total_ms = Vec::new();
        let mut bytes = Vec::new();
        for _ in 0..5 {
            let whole = std::time::Instant::now();
            let clock = std::time::Instant::now();
            let signed = runtime
                .block_on(async { client.sign(&request).await })
                .expect("vendor sign");
            sign_ms.push(clock.elapsed().as_millis() as u64);
            let clock = std::time::Instant::now();
            bytes = runtime
                .block_on(async { client.fetch(&signed).await })
                .expect("vendor fetch");
            fetch_ms.push(clock.elapsed().as_millis() as u64);
            total_ms.push(whole.elapsed().as_millis() as u64);
        }
        assert!(bytes.len() > 1000, "{} bytes", bytes.len());
        assert_eq!(&bytes[..2], &[0xff, 0xd8]);
        println!(
            "vendor picture bytes={} sign_ms={sign_ms:?} fetch_ms={fetch_ms:?} total_ms={total_ms:?} sign_median={} fetch_median={} total_median={} total_max={}",
            bytes.len(),
            median(&sign_ms),
            median(&fetch_ms),
            median(&total_ms),
            worst(&total_ms)
        );
    }

    fn median(values: &[u64]) -> u64 {
        let mut sorted = values.to_vec();
        sorted.sort_unstable();
        sorted[sorted.len() / 2]
    }

    fn worst(values: &[u64]) -> u64 {
        values.iter().copied().max().unwrap_or_default()
    }

    #[test]
    fn a_signed_target_must_be_https_on_storage() {
        let file = parse_sign(REPLY).expect("signed file");
        let url = target(&file, false).expect("vendor target");
        assert_eq!(
            url.host_str(),
            Some("oss-ap-id-pic-01.oss-ap-southeast-5.aliyuncs.com")
        );
        assert!(target(&file, true).is_ok());

        let plain = SignedFile {
            schema: "http".to_string(),
            ..file.clone()
        };
        assert!(target(&plain, false).is_err());
        assert!(target(&plain, true).is_ok());

        for host in [
            "evil.example.com",
            "notaliyuncs.com",
            "evil.example/x.aliyuncs.com",
            "evil.example?.aliyuncs.com",
            "evil.example#.aliyuncs.com",
            "evil.example\\.aliyuncs.com",
        ] {
            let hostile = SignedFile {
                host: host.to_string(),
                ..file.clone()
            };
            let checked = target(&hostile, false);
            assert!(checked.is_err(), "{host} was accepted as {checked:?}");
        }

        let relative = SignedFile {
            url: "objects/x.jpeg".to_string(),
            ..file.clone()
        };
        let slipped = target(&relative, false);
        assert!(
            slipped
                .as_ref()
                .map(|url| url.host_str() != Some(file.host.as_str()))
                .unwrap_or(true),
            "a relative path moved the host: {slipped:?}"
        );
    }

    #[test]
    fn endpoint_accepts_a_bare_host() {
        assert_eq!(
            CssClient::new("access-css.secu100.net").endpoint(),
            "http://access-css.secu100.net:6614/"
        );
        assert_eq!(
            CssClient::new("http://access-css.secu100.net/").endpoint(),
            "http://access-css.secu100.net:6614/"
        );
    }
}
