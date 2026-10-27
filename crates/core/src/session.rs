use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::provider::VendorSession;

struct Entry {
    session: VendorSession,
    expires: Instant,
}

#[derive(Default)]
pub struct Store {
    inner: Mutex<HashMap<String, Entry>>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, id: String, session: VendorSession, ttl: Duration) {
        let mut guard = self.inner.lock().expect("session store poisoned");
        guard.retain(|_, e| e.expires > Instant::now());
        guard.insert(
            id,
            Entry {
                session,
                expires: Instant::now() + ttl,
            },
        );
    }

    pub fn get(&self, id: &str) -> Option<VendorSession> {
        let mut guard = self.inner.lock().expect("session store poisoned");
        match guard.get(id) {
            Some(e) if e.expires > Instant::now() => Some(e.session.clone()),
            _ => {
                guard.remove(id);
                None
            }
        }
    }

    pub fn remove_where<F>(&self, predicate: F) -> usize
    where
        F: Fn(&VendorSession) -> bool,
    {
        let mut guard = self.inner.lock().expect("session store poisoned");
        let before = guard.len();
        guard.retain(|_, entry| !predicate(&entry.session));
        before - guard.len()
    }

    pub fn remove(&self, id: &str) {
        self.inner
            .lock()
            .expect("session store poisoned")
            .remove(id);
    }
}

pub fn new_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| format!("{:x}", rng.gen_range(0u8..16)))
        .collect()
}

#[cfg(test)]
mod removal_tests {
    use super::*;

    fn session(token: &str) -> VendorSession {
        VendorSession {
            account: "a@b.c".to_string(),
            token: Some(token.to_string()),
        }
    }

    #[test]
    fn removes_every_session_that_carries_the_same_token() {
        let store = Store::new();
        store.put("one".to_string(), session("t1"), Duration::from_secs(60));
        store.put("two".to_string(), session("t1"), Duration::from_secs(60));
        store.put("three".to_string(), session("t2"), Duration::from_secs(60));
        let removed = store.remove_where(|entry| entry.token.as_deref() == Some("t1"));
        assert_eq!(removed, 2);
        assert!(store.get("one").is_none());
        assert!(store.get("two").is_none());
        assert!(store.get("three").is_some());
    }
}
