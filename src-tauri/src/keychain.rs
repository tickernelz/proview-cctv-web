#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Mutex;

use keyring::Entry;
use serde::{Deserialize, Serialize};

pub const SERVICE: &str = "proview-cctv";
pub const DEFAULT_PROFILE: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub user: String,
    pub password: String,
}

pub trait SecretStore: Send + Sync {
    fn put(&self, profile: &str, credentials: &Credentials) -> Result<(), String>;
    fn get(&self, profile: &str) -> Result<Option<Credentials>, String>;
    fn delete(&self, profile: &str) -> Result<(), String>;
}

#[derive(Serialize, Deserialize)]
struct Stored {
    user: String,
    password: String,
}

pub struct OsKeychain;

impl OsKeychain {
    fn entry(profile: &str) -> Result<Entry, String> {
        Entry::new(SERVICE, profile).map_err(|error| error.to_string())
    }
}

impl SecretStore for OsKeychain {
    fn put(&self, profile: &str, credentials: &Credentials) -> Result<(), String> {
        let body = serde_json::to_string(&Stored {
            user: credentials.user.clone(),
            password: credentials.password.clone(),
        })
        .map_err(|error| error.to_string())?;
        Self::entry(profile)?
            .set_password(&body)
            .map_err(|error| error.to_string())
    }

    fn get(&self, profile: &str) -> Result<Option<Credentials>, String> {
        match Self::entry(profile)?.get_password() {
            Ok(body) => {
                let stored: Stored =
                    serde_json::from_str(&body).map_err(|error| error.to_string())?;
                Ok(Some(Credentials {
                    user: stored.user,
                    password: stored.password,
                }))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn delete(&self, profile: &str) -> Result<(), String> {
        match Self::entry(profile)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<HashMap<String, Credentials>>,
}

#[cfg(test)]
impl SecretStore for MemoryStore {
    fn put(&self, profile: &str, credentials: &Credentials) -> Result<(), String> {
        self.inner
            .lock()
            .expect("credential store poisoned")
            .insert(profile.to_string(), credentials.clone());
        Ok(())
    }

    fn get(&self, profile: &str) -> Result<Option<Credentials>, String> {
        Ok(self
            .inner
            .lock()
            .expect("credential store poisoned")
            .get(profile)
            .cloned())
    }

    fn delete(&self, profile: &str) -> Result<(), String> {
        self.inner
            .lock()
            .expect("credential store poisoned")
            .remove(profile);
        Ok(())
    }
}

pub fn read_stdin_credentials() -> Result<Credentials, String> {
    let mut raw = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut raw)
        .map_err(|error| error.to_string())?;
    let stored: Stored = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    if stored.user.trim().is_empty() {
        return Err("the user field is empty".to_string());
    }
    Ok(Credentials {
        user: stored.user,
        password: stored.password,
    })
}

#[cfg(test)]
mod tests {
    use super::{Credentials, MemoryStore, SecretStore};

    fn credentials() -> Credentials {
        Credentials {
            user: "admin".to_string(),
            password: "device-password".to_string(),
        }
    }

    #[test]
    fn a_stored_credential_round_trips_and_deletes() {
        let store = MemoryStore::default();
        assert_eq!(store.get("default").expect("readable"), None);
        store.put("default", &credentials()).expect("storable");
        assert_eq!(store.get("default").expect("readable"), Some(credentials()));
        assert_eq!(store.get("other").expect("readable"), None);
        store.delete("default").expect("deletable");
        assert_eq!(store.get("default").expect("readable"), None);
    }

    #[test]
    fn deleting_an_absent_profile_is_not_an_error() {
        let store = MemoryStore::default();
        store.delete("missing").expect("idempotent delete");
    }
}
