use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use openssl::symm::{decrypt_aead, encrypt_aead, Cipher};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};

pub const STORE_FILE: &str = "device-tokens.json";
pub const KEY_FILE: &str = "device-token.key";
const VERSION: u32 = 1;
const NONCE: usize = 12;

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    nonce: String,
    tag: String,
    payload: String,
}

pub struct DeviceTokenStore {
    dir: Option<PathBuf>,
    key: Option<[u8; 32]>,
}

impl DeviceTokenStore {
    pub fn load(dir: Option<&Path>) -> Self {
        let dir = dir
            .filter(|value| !value.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .or_else(|| {
                env::var("PROVIEW_DATA_DIR")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(PathBuf::from)
            });
        let key = dir.as_deref().and_then(read_or_create_key);
        Self { dir, key }
    }

    pub fn enabled(&self) -> bool {
        self.dir.is_some() && self.key.is_some()
    }

    pub fn get(&self, serial: &str) -> Option<String> {
        let key = self.key?;
        let entry = self
            .file()
            .and_then(|file| file.entries.get(serial).cloned())?;
        let nonce = STANDARD.decode(entry.nonce.trim()).ok()?;
        let tag = STANDARD.decode(entry.tag.trim()).ok()?;
        let payload = STANDARD.decode(entry.payload.trim()).ok()?;
        let plain = decrypt_aead(
            Cipher::aes_256_gcm(),
            &key,
            Some(&nonce),
            serial.as_bytes(),
            &payload,
            &tag,
        )
        .ok()?;
        String::from_utf8(plain).ok()
    }

    pub fn put(&self, serial: &str, token: &str) -> Result<(), String> {
        let key = self.key.ok_or_else(|| {
            "no device token key: set PROVIEW_DATA_DIR to a writable directory".to_string()
        })?;
        let mut nonce = [0u8; NONCE];
        OsRng.fill_bytes(&mut nonce);
        let mut tag = [0u8; 16];
        let payload = encrypt_aead(
            Cipher::aes_256_gcm(),
            &key,
            Some(&nonce),
            serial.as_bytes(),
            token.as_bytes(),
            &mut tag,
        )
        .map_err(|error| format!("device token sealing failed: {error}"))?;
        let mut file = self.file().unwrap_or_default();
        file.version = VERSION;
        file.entries.insert(
            serial.to_string(),
            Entry {
                nonce: STANDARD.encode(nonce),
                tag: STANDARD.encode(tag),
                payload: STANDARD.encode(payload),
            },
        );
        self.persist(&file)
    }

    pub fn forget(&self, serial: &str) -> Result<bool, String> {
        let mut file = self.file().unwrap_or_default();
        let removed = file.entries.remove(serial).is_some();
        if removed {
            self.persist(&file)?;
        }
        Ok(removed)
    }

    pub fn serials(&self) -> Vec<String> {
        self.file()
            .map(|file| file.entries.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|dir| dir.join(STORE_FILE))
    }

    fn file(&self) -> Option<StoreFile> {
        let raw = fs::read_to_string(self.path()?).ok()?;
        serde_json::from_str(&raw).ok()
    }

    fn persist(&self, file: &StoreFile) -> Result<(), String> {
        let path = self
            .path()
            .ok_or_else(|| "no device token store path".to_string())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("device token directory: {error}"))?;
        }
        let raw = serde_json::to_string_pretty(file)
            .map_err(|error| format!("device token encode: {error}"))?;
        let temporary = path.with_extension("json.new");
        fs::write(&temporary, raw).map_err(|error| format!("device token write: {error}"))?;
        set_private(&temporary)?;
        fs::rename(&temporary, &path).map_err(|error| format!("device token rename: {error}"))
    }
}

fn read_or_create_key(dir: &Path) -> Option<[u8; 32]> {
    let path = dir.join(KEY_FILE);
    if let Ok(raw) = fs::read_to_string(&path) {
        let decoded = STANDARD.decode(raw.trim()).ok()?;
        if decoded.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&decoded);
            return Some(key);
        }
        return None;
    }
    fs::create_dir_all(dir).ok()?;
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    fs::write(&path, STANDARD.encode(key)).ok()?;
    set_private(&path).ok()?;
    Some(key)
}

fn set_private(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("device token permission: {error}"))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let mut suffix = [0u8; 6];
        OsRng.fill_bytes(&mut suffix);
        let dir = env::temp_dir().join(format!("proview-tokens-{}", STANDARD.encode(suffix)));
        fs::create_dir_all(&dir).expect("scratch directory");
        dir
    }

    #[test]
    fn a_token_round_trips_through_the_sealed_store() {
        let dir = scratch();
        let store = DeviceTokenStore::load(Some(&dir));
        store
            .put("7f92970ddf5282743hrc", "token-value")
            .expect("put");
        assert_eq!(
            store.get("7f92970ddf5282743hrc").as_deref(),
            Some("token-value")
        );
        let raw = fs::read_to_string(dir.join(STORE_FILE)).expect("store file");
        assert!(
            !raw.contains("token-value"),
            "the file must not carry the token"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_store_entry_does_not_answer_for_another_serial() {
        let dir = scratch();
        let store = DeviceTokenStore::load(Some(&dir));
        store.put("serial-one", "token-value").expect("put");
        assert!(store.get("serial-two").is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn forgetting_a_device_removes_only_that_entry() {
        let dir = scratch();
        let store = DeviceTokenStore::load(Some(&dir));
        store.put("serial-one", "first").expect("put first");
        store.put("serial-two", "second").expect("put second");
        assert!(store.forget("serial-one").expect("forget"));
        assert!(store.get("serial-one").is_none());
        assert_eq!(store.get("serial-two").as_deref(), Some("second"));
        assert!(!store.forget("serial-one").expect("forget again"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_rotated_key_cannot_open_an_existing_entry() {
        let dir = scratch();
        let store = DeviceTokenStore::load(Some(&dir));
        store.put("serial-one", "token-value").expect("put");
        fs::remove_file(dir.join(KEY_FILE)).expect("drop key");
        let rotated = DeviceTokenStore::load(Some(&dir));
        assert!(rotated.get("serial-one").is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_empty_directory_is_not_a_store() {
        let store = DeviceTokenStore::load(Some(Path::new("")));
        assert!(!store.enabled());
        assert!(store.put("serial-one", "token-value").is_err());
    }
}
