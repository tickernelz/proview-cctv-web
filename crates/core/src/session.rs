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
