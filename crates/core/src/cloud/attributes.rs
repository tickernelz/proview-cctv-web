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
pub const SHARE_OUT_ENDPOINT: &str = "devicesTransOut";
pub const SHARE_PENDING_ENDPOINT: &str = "hasDevicesTrans";
pub const SHARE_INFO_ENDPOINT: &str = "devicesTransInfo";
pub const SHARE_ACCEPT_ENDPOINT: &str = "devicesTransIn";
pub const SHARE_CANCEL_ENDPOINT: &str = "devicesTransCancel";

const ACCEPT: &str = "text/html, image/gif, image/jpeg, *; q=.2, */*; q=.2";
const USER_AGENT: &str = "proview-cctv-web/0.3";

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

    async fn call(&self, endpoint: &str, body: Option<String>, access: &str) -> CloudResult<Value> {
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
        if plain.trim().is_empty() {
            return Ok(json!({}));
        }
        let envelope: Envelope = serde_json::from_str(&plain)
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

#[cfg(test)]
mod tests {
    use super::*;

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
