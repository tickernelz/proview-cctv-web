use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::client::{CloudError, CloudResult};
use super::envelope::{
    open, seal, signed_request, url_encode, Identity, COUNTER_WRAP, ENVELOPE_BASE, SIGNED_VERSION,
};
use super::types::{user_message, CloudDevice, Envelope, CODE_TOKEN_EXPIRED};

pub const RENAME_ENDPOINT: &str = "mdedit";
pub const LOGIN_PASSWORD_ENDPOINT: &str = "mdeditPass";
pub const USER_SEARCH_ENDPOINT: &str = "usersearch";
pub const SHARE_ACCEPT_DEVICE_ENDPOINT: &str = "mdshareaccept";
pub const SHARE_BASE: &str = "https://as-ams.jftechws.com";
pub const SHARE_OUT_ENDPOINT: &str = "devicesTransOut";
pub const SHARE_PENDING_ENDPOINT: &str = "hasDevicesTrans";
pub const SHARE_INFO_ENDPOINT: &str = "devicesTransInfo";
pub const SHARE_ACCEPT_ENDPOINT: &str = "devicesTransIn";
pub const SHARE_CANCEL_ENDPOINT: &str = "devicesTransCancel";
pub const SHARE_ADD_ENDPOINT: &str = "mdshareadd";
pub const SHARE_VERSION: &str = "v1";
pub const SHARE_MINE_ENDPOINT: &str = "mdsharemylist";
pub const SHARE_RECEIVED_ENDPOINT: &str = "mdsharelist";
pub const SHARE_PERMISSION_ENDPOINT: &str = "mdsharesetpermission";
pub const SHARE_DELETE_ENDPOINT: &str = "mdsharedel";
pub const SHARE_REFUSE_ENDPOINT: &str = "mdsharerefuse";
pub const APP_INFO_SAVE_ENDPOINT: &str = "appInfoSave";
pub const APP_INFO_QUERY_ENDPOINT: &str = "appInfoQueryLimit";
pub const SHARE_QR_BASE: &str = "https://d.xmeye.net/CSee";
pub const SHARE_QR_LIFETIME_MS: u64 = 1_800_000;
pub const SHARE_PERMISSIONS: [(&str, &str, bool); 6] = [
    ("DP_PTZ", "Pan and tilt", true),
    ("DP_Intercom", "Two-way talk", true),
    ("DP_LocalStorage", "Recordings on the card", true),
    ("DP_ModifyConfig", "Change settings", false),
    ("DP_AlarmPush", "Alarm messages", false),
    ("DP_ViewCloudVideo", "Cloud video", false),
];

pub fn permission_list(wanted: &[String]) -> CloudResult<String> {
    let mut picked: Vec<&str> = Vec::new();
    for item in wanted {
        let name = item.trim();
        if name.is_empty() {
            continue;
        }
        let known = SHARE_PERMISSIONS
            .iter()
            .find(|(id, _, _)| *id == name)
            .ok_or_else(|| CloudError::Rejected(format!("{name:?} is not a share permission")))?;
        if !picked.contains(&known.0) {
            picked.push(known.0);
        }
    }
    picked.sort_by_key(|id| {
        SHARE_PERMISSIONS
            .iter()
            .position(|(known, _, _)| known == id)
            .unwrap_or(usize::MAX)
    });
    Ok(picked.join(","))
}

pub fn dev_info_plain(serial: &str, user: &str, password: &str, kind: u32, time: i64) -> String {
    let user = if user.trim() == "admin" {
        ""
    } else {
        user.trim()
    };
    format!("{serial},{user},{password},{kind},{time}")
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShareLogin {
    pub account: String,
    pub password: String,
}

impl ShareLogin {
    pub fn new(account: &str, password: &str) -> Self {
        Self {
            account: account.trim().to_string(),
            password: password.to_string(),
        }
    }
}

pub fn share_query(login: &ShareLogin, fields: &[(&str, &str)]) -> String {
    let mut pairs: Vec<(&str, &str)> = Vec::with_capacity(fields.len() + 2);
    if !login.password.is_empty() && !login.account.is_empty() {
        pairs.push(("uname", login.account.as_str()));
        pairs.push(("upass", login.password.as_str()));
    }
    pairs.extend_from_slice(fields);
    pairs
        .iter()
        .map(|(key, value)| format!("{key}={}", url_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn share_path(endpoint: &str, signed: &super::envelope::Signed, query: &str) -> String {
    format!(
        "/{endpoint}/{SHARE_VERSION}/{}/{}.rs?{query}",
        signed.tsid, signed.sig
    )
}

fn form_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or_default();
                match u8::from_str_radix(hex, 16) {
                    Ok(value) => {
                        out.push(value);
                        index += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn share_reply_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        trimmed.to_string()
    } else {
        form_decode(trimmed)
    }
}

pub fn share_powers(blob: &str) -> String {
    json!({ "devInfo": blob }).to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareQrInfo {
    #[serde(rename = "devId")]
    pub dev_id: String,
    #[serde(rename = "devType")]
    pub dev_type: u32,
    #[serde(rename = "userId")]
    pub user_id: String,
    #[serde(rename = "loginName")]
    pub login_name: String,
    pub pwd: String,
    #[serde(rename = "shareTimes")]
    pub share_times: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub permissions: String,
}

pub fn share_qr_link(key: &str) -> String {
    format!("{SHARE_QR_BASE}?shareKey={}", url_encode(key.trim()))
}

pub fn share_key_of(payload: &str) -> Option<String> {
    let index = payload.find("shareKey=")?;
    let value = &payload[index + "shareKey=".len()..];
    let key = value.split(['&', '#']).next().unwrap_or(value).trim();
    (!key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    .then(|| key.to_string())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareEntry {
    pub share_id: String,
    pub serial: String,
    pub account: String,
    pub user_id: String,
    pub permissions: Vec<String>,
    pub state: String,
    pub device_name: String,
}

fn text_of(row: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| match row.get(*key) {
            Some(Value::String(text)) if !text.trim().is_empty() => Some(text.trim().to_string()),
            Some(Value::Number(number)) => Some(number.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

fn permissions_of(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter(|item| item["enabled"].as_bool() == Some(true))
            .filter_map(|item| item["label"].as_str())
            .map(|label| label.trim().to_string())
            .filter(|label| !label.is_empty())
            .collect(),
        Value::String(text) => text
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

pub fn share_entries(data: &Value) -> Vec<ShareEntry> {
    let rows: Vec<&Value> = match data {
        Value::Array(rows) => rows.iter().collect(),
        Value::Object(fields) => ["list", "shares", "data"]
            .iter()
            .find_map(|key| fields.get(*key).and_then(Value::as_array))
            .map(|rows| rows.iter().collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    rows.into_iter()
        .filter_map(|row| {
            let share_id = text_of(row, &["id", "shareId"]);
            if share_id.is_empty() {
                return None;
            }
            Some(ShareEntry {
                share_id,
                serial: text_of(row, &["uuid", "shareUuid", "devId"]),
                account: text_of(row, &["account", "acceptAccount"]),
                user_id: text_of(row, &["shareAccountId", "userId", "acceptId"]),
                permissions: permissions_of(&row["permissions"]),
                state: text_of(row, &["ret", "shareState", "state"]),
                device_name: text_of(row, &["devName", "nickname"]),
            })
        })
        .collect()
}

const ACCEPT: &str = "text/html, image/gif, image/jpeg, *; q=.2, */*; q=.2";
const USER_AGENT: &str = "proview-cctv-web/0.3";

fn envelope_reply(endpoint: &str, plain: &str) -> CloudResult<Value> {
    if plain.trim().is_empty() {
        return Ok(json!({}));
    }
    let envelope: Envelope = serde_json::from_str(plain)
        .map_err(|error| CloudError::Malformed(format!("{endpoint}: {error}")))?;
    if !envelope.is_ok() {
        return Err(if envelope.code == CODE_TOKEN_EXPIRED {
            CloudError::Expired
        } else {
            CloudError::Rejected(user_message(envelope.code, envelope.msg.as_deref()))
        });
    }
    Ok(json!({
        "code": envelope.code,
        "msg": envelope.msg,
        "data": envelope.data,
    }))
}

fn next_counter(raw: u32) -> u32 {
    raw % (COUNTER_WRAP - 1) + 1
}

pub struct AttributeClient {
    http: reqwest::Client,
    base: String,
    identity: Identity,
    counter: AtomicU32,
}

impl AttributeClient {
    pub fn new(identity: Identity) -> Self {
        Self::with_base(identity, ENVELOPE_BASE)
    }

    pub fn with_base(identity: Identity, base: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("attribute http client");
        let base = base.into().trim_end_matches('/').to_string();
        Self {
            http,
            base: if base.is_empty() {
                ENVELOPE_BASE.to_string()
            } else {
                base
            },
            identity,
            counter: AtomicU32::new(0),
        }
    }

    fn next(&self) -> super::envelope::Signed {
        let counter = next_counter(self.counter.fetch_add(1, Ordering::Relaxed));
        signed_request(&self.identity, counter, super::envelope::now_millis())
    }

    pub async fn call(
        &self,
        endpoint: &str,
        body: Option<String>,
        access: &str,
    ) -> CloudResult<Value> {
        let signed = self.next();
        let url = format!(
            "{}/{endpoint}/{}/{}/{}.rs",
            self.base, SIGNED_VERSION, signed.tsid, signed.sig
        );
        let mut request = self
            .http
            .post(url)
            .header("Accept", ACCEPT)
            .header("Accept-Charset", "utf-8")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("uuid", &self.identity.uuid)
            .header("appKey", &self.identity.app_key)
            .header("Authorization", access);
        if let Some(plain) = body {
            request = request.body(seal(&signed.key, &plain)?);
        }
        let response = request
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        if !status.is_success() {
            return Err(CloudError::Unreachable(format!("{endpoint} HTTP {status}")));
        }
        let plain = if text.trim().is_empty() {
            String::new()
        } else if let Ok(opened) = open(&signed.key, &text) {
            opened
        } else {
            text
        };
        envelope_reply(endpoint, &plain)
    }

    pub async fn call_share(
        &self,
        endpoint: &str,
        login: &ShareLogin,
        fields: &[(&str, &str)],
        access: &str,
    ) -> CloudResult<Value> {
        let signed = self.next();
        let url = format!(
            "{}{}",
            self.base,
            share_path(endpoint, &signed, &share_query(login, fields))
        );
        let response = self
            .http
            .post(url)
            .header("Accept", ACCEPT)
            .header("Accept-Charset", "utf-8")
            .header("uuid", &self.identity.uuid)
            .header("appKey", &self.identity.app_key)
            .header("Authorization", access)
            .header("Content-Length", "0")
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        if !status.is_success() {
            return Err(CloudError::Unreachable(format!("{endpoint} HTTP {status}")));
        }
        envelope_reply(endpoint, &share_reply_text(&text))
    }

    pub async fn rename(
        &self,
        access: &str,
        uuid: &str,
        account: &str,
        nickname: &str,
    ) -> CloudResult<Value> {
        let name = nickname.trim();
        if name.is_empty() {
            return Err(CloudError::Rejected("a device needs a name".to_string()));
        }
        let body = format!(
            "uuid={}&username={}&nickname={}",
            url_encode(uuid.trim()),
            url_encode(account.trim()),
            url_encode(name)
        );
        self.call(RENAME_ENDPOINT, Some(body), access).await
    }

    pub async fn change_login_password(
        &self,
        access: &str,
        uuid: &str,
        password: &str,
        new_password: &str,
    ) -> CloudResult<Value> {
        if new_password.trim().is_empty() {
            return Err(CloudError::Rejected(
                "the new device password is empty".to_string(),
            ));
        }
        let body = format!(
            "uuid={}&password={}&newPass={}&confirmPass={}",
            url_encode(uuid.trim()),
            url_encode(password),
            url_encode(new_password),
            url_encode(new_password)
        );
        self.call(LOGIN_PASSWORD_ENDPOINT, Some(body), access).await
    }

    pub async fn search_user(&self, access: &str, query: &str) -> CloudResult<Vec<UserSearchHit>> {
        let needle = query.trim();
        if needle.is_empty() {
            return Err(CloudError::Rejected(
                "a user search needs a query".to_string(),
            ));
        }
        let body = format!("search={}", url_encode(needle));
        let reply = self.call(USER_SEARCH_ENDPOINT, Some(body), access).await?;
        Ok(parse_user_hits(&reply["data"]))
    }

    pub async fn share_out(
        &self,
        access: &str,
        uuids: &[String],
        accept_id: &str,
    ) -> CloudResult<Value> {
        let target = accept_id.trim();
        if target.is_empty() {
            return Err(CloudError::Rejected(
                "the receiving account id is empty".to_string(),
            ));
        }
        let body = format!(
            "uuids={}&transInUserId={}",
            url_encode(&uuids.join(",")),
            url_encode(target)
        );
        self.call(SHARE_OUT_ENDPOINT, Some(body), access).await
    }

    pub async fn share_pending(&self, access: &str, uuids: &[String]) -> CloudResult<Value> {
        let body = format!("uuids={}", url_encode(&uuids.join(",")));
        self.call(SHARE_PENDING_ENDPOINT, Some(body), access).await
    }

    pub async fn share_info(&self, access: &str, trans_key: &str) -> CloudResult<Value> {
        let body = format!("transKey={}", url_encode(trans_key.trim()));
        self.call(SHARE_INFO_ENDPOINT, Some(body), access).await
    }

    pub async fn share_accept(&self, access: &str, trans_key: &str) -> CloudResult<Value> {
        let body = format!("transKey={}", url_encode(trans_key.trim()));
        self.call(SHARE_ACCEPT_ENDPOINT, Some(body), access).await
    }

    pub async fn share_accept_device(
        &self,
        access: &str,
        account: &str,
        password: &str,
        dev_id: &str,
    ) -> CloudResult<Value> {
        let target = dev_id.trim();
        if target.is_empty() {
            return Err(CloudError::Rejected(
                "accepting a share needs the device id".to_string(),
            ));
        }
        let body = format!(
            "uname={}&upass={}&devId={}",
            url_encode(account.trim()),
            url_encode(password),
            url_encode(target)
        );
        self.call(SHARE_ACCEPT_DEVICE_ENDPOINT, Some(body), access)
            .await
    }

    pub async fn share_cancel(
        &self,
        access: &str,
        trans_key: &str,
        uuids: &[String],
    ) -> CloudResult<Value> {
        let body = format!(
            "transKey={}&uuids={}",
            url_encode(trans_key.trim()),
            url_encode(&uuids.join(","))
        );
        self.call(SHARE_CANCEL_ENDPOINT, Some(body), access).await
    }
}

impl AttributeClient {
    pub async fn share_with_permissions(
        &self,
        access: &str,
        login: &ShareLogin,
        serial: &str,
        accept_id: &str,
        powers: &str,
        permissions: &str,
    ) -> CloudResult<Value> {
        let target = accept_id.trim();
        if target.is_empty() {
            return Err(CloudError::Rejected(
                "the receiving account id is empty".to_string(),
            ));
        }
        self.call_share(
            SHARE_ADD_ENDPOINT,
            login,
            &[
                ("shareUuid", serial.trim()),
                ("acceptId", target),
                ("powers", powers),
                ("permissions", permissions),
            ],
            access,
        )
        .await
    }

    pub async fn shares_of_device(
        &self,
        access: &str,
        login: &ShareLogin,
        serial: &str,
    ) -> CloudResult<Vec<ShareEntry>> {
        let reply = self
            .call_share(
                SHARE_MINE_ENDPOINT,
                login,
                &[("shareUuid", serial.trim())],
                access,
            )
            .await?;
        Ok(share_entries(&reply["data"]))
    }

    pub async fn shares_received(
        &self,
        access: &str,
        login: &ShareLogin,
    ) -> CloudResult<Vec<ShareEntry>> {
        let reply = self
            .call_share(SHARE_RECEIVED_ENDPOINT, login, &[], access)
            .await?;
        Ok(share_entries(&reply["data"]))
    }

    pub async fn set_share_permissions(
        &self,
        access: &str,
        login: &ShareLogin,
        share_id: &str,
        permissions: &str,
    ) -> CloudResult<Value> {
        self.call_share(
            SHARE_PERMISSION_ENDPOINT,
            login,
            &[("shareId", share_id.trim()), ("permissions", permissions)],
            access,
        )
        .await
    }

    pub async fn revoke_share(
        &self,
        access: &str,
        login: &ShareLogin,
        share_id: &str,
    ) -> CloudResult<Value> {
        self.call_share(
            SHARE_DELETE_ENDPOINT,
            login,
            &[("devId", share_id.trim())],
            access,
        )
        .await
    }

    pub async fn accept_permission_share(
        &self,
        access: &str,
        login: &ShareLogin,
        share_id: &str,
    ) -> CloudResult<Value> {
        self.call_share(
            SHARE_ACCEPT_DEVICE_ENDPOINT,
            login,
            &[("devId", share_id.trim())],
            access,
        )
        .await
    }

    pub async fn refuse_share(
        &self,
        access: &str,
        login: &ShareLogin,
        share_id: &str,
    ) -> CloudResult<Value> {
        self.call_share(
            SHARE_REFUSE_ENDPOINT,
            login,
            &[("devId", share_id.trim())],
            access,
        )
        .await
    }

    pub async fn save_share_info(
        &self,
        access: &str,
        blob: &str,
        lifetime_ms: u64,
    ) -> CloudResult<(String, i64)> {
        let body = format!("info={}&expireTime={lifetime_ms}", url_encode(blob));
        let reply = self
            .call(APP_INFO_SAVE_ENDPOINT, Some(body), access)
            .await?;
        let key = reply["data"]["key"]
            .as_str()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| CloudError::Malformed("appInfoSave answered without a key".to_string()))?
            .to_string();
        let expires = reply["data"]["expireTime"].as_i64().unwrap_or_default();
        Ok((key, expires))
    }

    pub async fn share_info_of_key(&self, access: &str, key: &str) -> CloudResult<String> {
        let body = format!("key={}", url_encode(key.trim()));
        let reply = self
            .call(APP_INFO_QUERY_ENDPOINT, Some(body), access)
            .await?;
        reply["data"]
            .as_str()
            .map(str::trim)
            .filter(|blob| !blob.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                CloudError::Rejected("the share code has expired or was never issued".to_string())
            })
    }
}

pub fn trans_key_of(reply: &Value) -> Option<String> {
    let data = reply.get("data").unwrap_or(reply);
    for candidate in [data.get("transKey"), data.get("key"), data.get("shareKey")] {
        if let Some(text) = candidate.and_then(Value::as_str) {
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
    }
    data.get("transInfos")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("transKey"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organisation {
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub favourite: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeStore {
    #[serde(default)]
    pub devices: BTreeMap<String, Organisation>,
}

impl AttributeStore {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{} is not readable: {error}", path.display()),
                )
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let temp = path.with_extension(format!("{}.{unique}.tmp", std::process::id()));
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, path)
    }

    pub fn get(&self, uuid: &str) -> Organisation {
        self.devices.get(uuid).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, uuid: &str, organisation: Organisation) {
        if organisation == Organisation::default() {
            self.devices.remove(uuid);
        } else {
            self.devices.insert(uuid.to_string(), organisation);
        }
    }

    pub fn groups(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .devices
            .values()
            .map(|organisation| organisation.group.clone())
            .filter(|group| !group.trim().is_empty())
            .collect();
        names.sort();
        names.dedup();
        names
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceAttributes {
    pub uuid: String,
    pub nickname: String,
    pub group: String,
    pub favourite: bool,
    pub shared_accounts: u64,
    pub administrator: bool,
    pub extension: String,
}

impl DeviceAttributes {
    pub fn of(device: &CloudDevice, organisation: &Organisation) -> Self {
        let shared = device
            .extra
            .get("numberOfSharedAccounts")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let administrator = device
            .extra
            .get("mAccount")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let extension = device
            .extra
            .get("ext")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Self {
            uuid: device.uuid.clone(),
            nickname: device.display_name().to_string(),
            group: organisation.group.clone(),
            favourite: organisation.favourite,
            shared_accounts: shared,
            administrator,
            extension,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSearchHit {
    pub account: String,
    pub id: String,
}

fn parse_user_hits(data: &Value) -> Vec<UserSearchHit> {
    let rows = match data {
        Value::Array(rows) => rows.clone(),
        Value::String(raw) => serde_json::from_str::<Vec<Value>>(raw).unwrap_or_default(),
        Value::Object(map) => map
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    rows.iter()
        .filter_map(|row| {
            let account = row
                .get("account")
                .and_then(Value::as_str)?
                .trim()
                .to_string();
            let id = row.get("id").and_then(Value::as_str)?.trim().to_string();
            if account.is_empty() || id.is_empty() {
                return None;
            }
            Some(UserSearchHit { account, id })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_keep_the_app_order_drop_repeats_and_refuse_strangers() {
        let picked = permission_list(&[
            "DP_ViewCloudVideo".into(),
            " DP_PTZ ".into(),
            "".into(),
            "DP_PTZ".into(),
        ])
        .expect("known");
        assert_eq!(picked, "DP_PTZ,DP_ViewCloudVideo");
        assert_eq!(permission_list(&[]).expect("none"), "");
        assert!(permission_list(&["DP_Root".into()]).is_err());
        let defaults: Vec<&str> = SHARE_PERMISSIONS
            .iter()
            .filter(|(_, _, on)| *on)
            .map(|(id, _, _)| *id)
            .collect();
        assert_eq!(defaults, ["DP_PTZ", "DP_Intercom", "DP_LocalStorage"]);
    }

    #[test]
    fn device_info_blanks_the_admin_user_like_the_native_encoder() {
        assert_eq!(dev_info_plain("SN1", "admin", "pw", 0, 9), "SN1,,pw,0,9");
        assert_eq!(
            dev_info_plain("SN1", "guard", "pw", 0, 9),
            "SN1,guard,pw,0,9"
        );
        let powers: Value = serde_json::from_str(&share_powers("blob")).expect("json");
        assert_eq!(powers, json!({ "devInfo": "blob" }));
    }

    #[test]
    fn a_share_key_is_read_from_the_link_and_nothing_else_is() {
        let link = share_qr_link("Ab3-x_9.z");
        assert_eq!(link, "https://d.xmeye.net/CSee?shareKey=Ab3-x_9.z");
        assert_eq!(share_key_of(&link).as_deref(), Some("Ab3-x_9.z"));
        assert_eq!(
            share_key_of("https://d.xmeye.net/CSee?shareKey=k1&x=2").as_deref(),
            Some("k1")
        );
        assert_eq!(share_key_of("https://d.xmeye.net/CSee?shareInfo=abc"), None);
        assert_eq!(share_key_of("shareKey=a/b"), None);
        assert_eq!(share_key_of("shareKey="), None);
    }

    #[test]
    fn the_share_request_is_the_apps_retrofit_call() {
        let identity = Identity::new(
            "app-uuid-0001",
            "app-key-0001",
            "0123456789abcdef0123456789abcdef",
        );
        let signed = signed_request(&identity, 1, 1_793_351_709_004);
        let query = share_query(
            &ShareLogin::new("owner@example.com", "pw"),
            &[
                ("shareUuid", "SN1"),
                ("acceptId", "YWI="),
                ("powers", "{\"devInfo\":\"z1evz\"}"),
                ("permissions", "DP_PTZ,DP_LocalStorage"),
            ],
        );
        assert_eq!(
            share_path(SHARE_ADD_ENDPOINT, &signed, &query),
            "/mdshareadd/v1/00000011793351709004/ad0721c593236318331a9d2be5bba600.rs?uname=owner%40example.com&upass=pw&shareUuid=SN1&acceptId=YWI%3D&powers=%7B%22devInfo%22%3A%22z1evz%22%7D&permissions=DP_PTZ%2CDP_LocalStorage"
        );
        assert_eq!(
            share_query(&ShareLogin::new("owner@example.com", ""), &[("devId", "s")]),
            "devId=s"
        );
    }

    #[test]
    fn a_share_reply_is_read_url_encoded_or_plain() {
        assert_eq!(
            share_reply_text("%7B%22code%22%3A2000%2C%22msg%22%3A%22a+b%22%7D"),
            "{\"code\":2000,\"msg\":\"a b\"}"
        );
        assert_eq!(share_reply_text(" {\"code\":2000} "), "{\"code\":2000}");
    }

    #[test]
    fn a_live_share_row_reads_enabled_labels_ret_and_the_peer_id() {
        let rows = share_entries(&json!([{
            "id": "c9092811acaa145439c1281df08eae9a",
            "uuid": "SN1",
            "ret": 0,
            "account": "zh***@gmail.com",
            "shareAccountId": "as92",
            "username": "admin",
            "permissions": [
                { "label": "DP_AlarmPush", "enabled": true },
                { "label": "DP_CLoudConfig", "enabled": false },
                { "label": "DP_PTZ", "enabled": true },
                { "label": "DP_VideoCall", "enabled": false }
            ]
        }]));
        assert_eq!(rows[0].share_id, "c9092811acaa145439c1281df08eae9a");
        assert_eq!(rows[0].permissions, vec!["DP_AlarmPush", "DP_PTZ"]);
        assert_eq!(rows[0].state, "0");
        assert_eq!(rows[0].user_id, "as92");
        assert_eq!(rows[0].account, "zh***@gmail.com");
    }

    #[test]
    fn share_rows_read_ids_accounts_and_split_permissions() {
        let rows = share_entries(&json!([
            { "id": 7, "uuid": "SN1", "account": "a@x", "userId": "u1", "permissions": "DP_PTZ, DP_Intercom", "shareState": 1, "devName": "Gate" },
            { "uuid": "SN2" }
        ]));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].share_id, "7");
        assert_eq!(rows[0].permissions, vec!["DP_PTZ", "DP_Intercom"]);
        assert_eq!(rows[0].state, "1");
        assert_eq!(rows[0].device_name, "Gate");
        assert!(share_entries(&json!("")).is_empty());
        assert_eq!(
            share_entries(&json!({ "list": [{ "shareId": "s" }] }))[0].share_id,
            "s"
        );
    }

    #[test]
    fn a_share_code_blob_round_trips_through_the_qr_parser() {
        let info = ShareQrInfo {
            dev_id: "SN00000001".into(),
            dev_type: 0,
            user_id: "owner".into(),
            login_name: "admin".into(),
            pwd: "pw".into(),
            share_times: 1_793_000_000,
            permissions: "DP_PTZ".into(),
        };
        let text = serde_json::to_string(&info).expect("json");
        assert!(text.contains("\"devId\":\"SN00000001\""), "{text}");
        assert!(text.contains("\"shareTimes\":1793000000"), "{text}");
        let blob = super::super::bind::encode_qr_blob(&text, 211);
        let device =
            super::super::bind::parse_qr(&format!("https://d.xmeye.net/CSee?shareInfo={blob}"))
                .expect("parses");
        assert_eq!(device.serial, "SN00000001");
        assert_eq!(device.password, "pw");
        assert_eq!(device.share_time, Some(1_793_000_000));
    }

    #[test]
    fn user_hits_are_read_from_an_array_a_string_and_a_list() {
        let direct = json!([{"account": "one@example.com", "id": "u-1"}]);
        assert_eq!(
            parse_user_hits(&direct),
            vec![UserSearchHit {
                account: "one@example.com".to_string(),
                id: "u-1".to_string(),
            }]
        );
        let encoded = Value::String(r#"[{"account":"two@example.com","id":"u-2"}]"#.to_string());
        assert_eq!(parse_user_hits(&encoded)[0].id, "u-2");
        let listed = json!({"list": [{"account": "three@example.com", "id": "u-3"}]});
        assert_eq!(parse_user_hits(&listed)[0].account, "three@example.com");
        let partial = json!([{"account": "four@example.com"}, {"id": "u-5"}]);
        assert!(parse_user_hits(&partial).is_empty());
    }

    #[test]
    fn counter_wraps_inside_the_signed_window() {
        assert_eq!(next_counter(0), 1);
        assert_eq!(next_counter(COUNTER_WRAP - 1), 1);
        assert_eq!(next_counter(COUNTER_WRAP - 2), COUNTER_WRAP - 1);
    }

    #[test]
    fn a_store_round_trips_through_its_file() {
        let dir = std::env::temp_dir().join(format!("pvw-attrs-{}", std::process::id()));
        let path = dir.join("device-groups.json");
        let mut store = AttributeStore::default();
        store.set(
            "DEV1",
            Organisation {
                group: "pintu".to_string(),
                favourite: true,
            },
        );
        store.save(&path).unwrap();
        let loaded = AttributeStore::load(&path).unwrap();
        assert_eq!(loaded.get("DEV1").group, "pintu");
        assert!(loaded.get("DEV1").favourite);
        assert_eq!(loaded.groups(), vec!["pintu".to_string()]);
        assert_eq!(loaded.get("dev2"), Organisation::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn clearing_an_organisation_removes_the_row() {
        let mut store = AttributeStore::default();
        store.set(
            "DEV1",
            Organisation {
                group: "pintu".to_string(),
                favourite: false,
            },
        );
        store.set("DEV1", Organisation::default());
        assert!(store.devices.is_empty());
    }

    #[test]
    fn a_missing_file_reads_as_empty() {
        let path = std::env::temp_dir().join("pvw-attrs-missing/store.json");
        let store = AttributeStore::load(&path).unwrap();
        assert!(store.devices.is_empty());
    }

    #[test]
    fn a_truncated_file_is_an_error_not_an_empty_store() {
        let dir = std::env::temp_dir().join(format!("pvw-attrs-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("device-groups.json");
        std::fs::write(&path, "{\"devices\": {\"A\": {\"group\": \"pintu\"").unwrap();
        let error =
            AttributeStore::load(&path).expect_err("a truncated file must not read as empty");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        let kept = std::fs::read_to_string(&path).unwrap();
        assert!(kept.contains("pintu"), "the file is left untouched: {kept}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_save_never_leaves_a_temporary_file_behind() {
        let dir = std::env::temp_dir().join(format!("pvw-attrs-temp-{}", std::process::id()));
        let path = dir.join("device-groups.json");
        let mut store = AttributeStore::default();
        store.set(
            "DEV1",
            Organisation {
                group: "pintu".to_string(),
                favourite: false,
            },
        );
        store.save(&path).unwrap();
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn attributes_read_the_shared_count_and_admin_flag() {
        let device: CloudDevice = serde_json::from_value(json!({
            "uuid": "SN-1",
            "nickname": "pintu",
            "numberOfSharedAccounts": 3,
            "mAccount": true,
            "ext": "just_test_ext"
        }))
        .unwrap();
        let attributes = DeviceAttributes::of(
            &device,
            &Organisation {
                group: "pintu".to_string(),
                favourite: true,
            },
        );
        assert_eq!(attributes.shared_accounts, 3);
        assert!(attributes.administrator);
        assert_eq!(attributes.extension, "just_test_ext");
        assert_eq!(attributes.group, "pintu");
        assert!(attributes.favourite);
    }

    #[test]
    fn a_transfer_key_is_found_under_the_reply_we_expect() {
        let direct = json!({ "code": 2000, "data": { "transKey": "abc" } });
        assert_eq!(trans_key_of(&direct), Some("abc".to_string()));
        let nested = json!({ "data": { "transInfos": [{ "transKey": "xyz" }] } });
        assert_eq!(trans_key_of(&nested), Some("xyz".to_string()));
        assert_eq!(trans_key_of(&json!({ "data": {} })), None);
    }
}
