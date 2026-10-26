use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::types::CloudSession;

struct Entry {
    session: CloudSession,
    expires: Instant,
}

pub struct CloudSessions {
    ttl: Duration,
    inner: Mutex<HashMap<String, Entry>>,
}

impl CloudSessions {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    pub fn put(&self, id: String, session: CloudSession) {
        let mut guard = self.inner.lock().expect("cloud session store poisoned");
        guard.retain(|_, entry| entry.expires > Instant::now());
        guard.insert(
            id,
            Entry {
                session,
                expires: Instant::now() + self.ttl,
            },
        );
    }

    pub fn get(&self, id: &str) -> Option<CloudSession> {
        let mut guard = self.inner.lock().expect("cloud session store poisoned");
        match guard.get(id) {
            Some(entry) if entry.expires > Instant::now() => Some(entry.session.clone()),
            _ => {
                guard.remove(id);
                None
            }
        }
    }

    pub fn remove(&self, id: &str) {
        self.inner
            .lock()
            .expect("cloud session store poisoned")
            .remove(id);
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("cloud session store poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub fn new_session_id() -> String {
    crate::session::new_id()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> CloudSession {
        CloudSession {
            account: "a@b.c".to_string(),
            access_token: "token-value".to_string(),
        }
    }

    #[test]
    fn stores_and_returns_a_session() {
        let store = CloudSessions::new(Duration::from_secs(60));
        store.put("id-1".to_string(), session());
        assert_eq!(store.get("id-1").unwrap().token(), "token-value");
    }

    #[test]
    fn drops_expired_sessions() {
        let store = CloudSessions::new(Duration::ZERO);
        store.put("id-1".to_string(), session());
        assert!(store.get("id-1").is_none());
        assert!(store.is_empty());
    }

    #[test]
    fn removes_on_logout() {
        let store = CloudSessions::new(Duration::from_secs(60));
        store.put("id-1".to_string(), session());
        store.remove("id-1");
        assert!(store.get("id-1").is_none());
    }

    #[test]
    fn ids_are_unique_and_hex() {
        let first = new_session_id();
        let second = new_session_id();
        assert_ne!(first, second);
        assert_eq!(first.len(), 32);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
