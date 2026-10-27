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
    http: reqwest::Client,
}

impl CssClient {
    pub fn new(host: impl AsRef<str>) -> Self {
        let host = host.as_ref().trim();
        let host = host
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .trim_end_matches('/');
        let (host, port) = match host.rsplit_once(':') {
            Some((host, port)) if port.parse::<u16>().is_ok() => {
                (host, port.parse::<u16>().unwrap_or(CSS_PORT))
            }
            _ => (host, CSS_PORT),
        };
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self {
            endpoint: format!("http://{host}:{port}/"),
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
        let method = if file.method.trim().is_empty() {
            "GET"
        } else {
            file.method.trim()
        };
        let mut request = self.http.request(
            reqwest::Method::from_bytes(method.as_bytes())
                .map_err(|error| CloudError::Rejected(format!("signed method {method}: {error}")))?,
            file.address(),
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
        let text = String::from_utf8_lossy(&bytes).trim().to_string();
        Err(CloudError::Rejected(format!(
            "object storage refused {}: {text} ({status})",
            file.address()
        )))
    }
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
        assert_eq!(centre["Body"]["AuthCode"], "aaaaaaaa-4a1c2f27d01793117365801");
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
            assert!(request.contains("003_DEV0001_261027214632.jpeg"), "{request}");
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
            assert!(request.contains("date: tue, 27 oct 2026 16:09:33 gmt"), "{request}");
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
        let bytes = runtime
            .block_on(async { CssClient::from_env().picture(&request).await })
            .expect("vendor picture");
        assert!(bytes.len() > 1000, "{} bytes", bytes.len());
        assert_eq!(&bytes[..2], &[0xff, 0xd8]);
        println!("vendor picture bytes={} first={:02x?}", bytes.len(), &bytes[..4]);
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
