use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::provider::VendorSession;

struct Entry {
    session: VendorSession,
    expires: Instant,
    order: u64,
}

#[derive(Default, Clone)]
pub struct Store {
    inner: Arc<Mutex<HashMap<String, Entry>>>,
    order: Arc<AtomicU64>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, id: String, session: VendorSession, ttl: Duration) {
        let order = self.order.fetch_add(1, Ordering::Relaxed);
        let mut guard = self.inner.lock().expect("session store poisoned");
        guard.retain(|_, e| e.expires > Instant::now());
        guard.insert(
            id,
            Entry {
                session,
                expires: Instant::now() + ttl,
                order,
            },
        );
    }

    pub fn latest(&self) -> Option<(String, VendorSession)> {
        let mut guard = self.inner.lock().expect("session store poisoned");
        guard.retain(|_, e| e.expires > Instant::now());
        guard
            .iter()
            .max_by_key(|(_, entry)| entry.order)
            .map(|(id, entry)| (id.clone(), entry.session.clone()))
    }

    pub fn latest_remaining(&self) -> Option<Duration> {
        let mut guard = self.inner.lock().expect("session store poisoned");
        let now = Instant::now();
        guard.retain(|_, entry| entry.expires > now);
        guard
            .values()
            .max_by_key(|entry| entry.order)
            .map(|entry| entry.expires.saturating_duration_since(now))
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

    #[test]
    fn latest_is_the_most_recently_stored_live_session() {
        let store = Store::new();
        assert!(store.latest().is_none(), "an empty store has no session");
        store.put("one".to_string(), session("t1"), Duration::from_secs(60));
        store.put("two".to_string(), session("t2"), Duration::from_secs(60));
        let (id, latest) = store.latest().expect("a live session");
        assert_eq!(id, "two");
        assert_eq!(latest.token.as_deref(), Some("t2"));
        store.remove("two");
        let (id, _) = store.latest().expect("the older session is still live");
        assert_eq!(id, "one");
    }

    #[test]
    fn latest_skips_an_expired_session() {
        let store = Store::new();
        store.put("old".to_string(), session("t1"), Duration::ZERO);
        assert!(store.latest().is_none(), "an expired session is not live");
    }
}
