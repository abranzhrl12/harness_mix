//! Explicitly opt-in metadata snapshots. Never cache credentials, data queries or failures.
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};

pub(super) const CAPACITY: usize = 64;
const MAX_BYTES: usize = 2 * 1024 * 1024;
struct Entry {
    value: Value,
    created: Instant,
    ttl: Duration,
    bytes: usize,
}
#[derive(Default)]
pub(super) struct Cache {
    entries: HashMap<String, Entry>,
    bytes: usize,
}
impl Cache {
    fn prune(&mut self, now: Instant) {
        self.entries
            .retain(|_, entry| now.saturating_duration_since(entry.created) < entry.ttl);
        self.bytes = self.entries.values().map(|entry| entry.bytes).sum();
    }
    fn get_at(&mut self, key: &str, now: Instant) -> Option<(Value, u64)> {
        self.prune(now);
        self.entries.get(key).map(|entry| {
            (
                entry.value.clone(),
                now.saturating_duration_since(entry.created).as_millis() as u64,
            )
        })
    }
    fn remove(&mut self, key: &str) {
        if let Some(entry) = self.entries.remove(key) {
            self.bytes -= entry.bytes;
        }
    }
    fn insert_at(&mut self, key: String, value: Value, ttl: Duration, now: Instant) {
        self.prune(now);
        self.remove(&key);
        let bytes = value.to_string().len();
        if ttl.is_zero() || bytes > MAX_BYTES {
            return;
        }
        while self.entries.len() >= CAPACITY || self.bytes + bytes > MAX_BYTES {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.created)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.remove(&oldest);
        }
        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                value,
                created: now,
                ttl,
                bytes,
            },
        );
    }
}
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}
pub(super) fn get(key: &str) -> Option<(Value, u64)> {
    cache().lock().ok()?.get_at(key, Instant::now())
}
pub(super) fn invalidate(key: &str) {
    if let Ok(mut cache) = cache().lock() {
        cache.remove(key);
    }
}
pub(super) fn insert(key: String, value: Value, ttl: Duration) {
    if let Ok(mut cache) = cache().lock() {
        cache.insert_at(key, value, ttl, Instant::now());
    }
}

/// Coalesce concurrent describes of the same identity; unrelated tables never share a gate.
pub(super) fn gate(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    static GATES: OnceLock<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut gates = GATES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gates.retain(|_, gate| gate.strong_count() != 0);
    if let Some(gate) = gates.get(key).and_then(Weak::upgrade) {
        return gate;
    }
    let gate = Arc::new(tokio::sync::Mutex::new(()));
    if gates.len() < CAPACITY {
        gates.insert(key.into(), Arc::downgrade(&gate));
    }
    gate
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn expiration_invalidation_rotation_and_zero_ttl() {
        let mut cache = Cache::default();
        let now = Instant::now();
        cache.insert_at(
            "role-a".into(),
            json!({"column":"id"}),
            Duration::from_secs(1),
            now,
        );
        assert!(cache.get_at("role-a", now).is_some());
        assert!(cache.get_at("role-b", now).is_none());
        assert!(
            cache
                .get_at("role-a", now + Duration::from_secs(1))
                .is_none()
        );
        cache.insert_at("role-a".into(), json!({}), Duration::ZERO, now);
        assert!(cache.get_at("role-a", now).is_none());
        cache.insert_at("role-a".into(), json!({}), Duration::from_secs(1), now);
        cache.remove("role-a");
        assert!(cache.get_at("role-a", now).is_none());
    }
    #[test]
    fn entry_and_byte_capacity_are_bounded() {
        let mut cache = Cache::default();
        let now = Instant::now();
        for i in 0..100 {
            cache.insert_at(
                i.to_string(),
                json!("x".repeat(40000)),
                Duration::from_secs(60),
                now,
            );
        }
        assert!(cache.entries.len() <= CAPACITY);
        assert!(cache.bytes <= MAX_BYTES);
    }
    #[test]
    fn single_flight_gate_is_per_identity() {
        let first = gate("first");
        let same = gate("first");
        let other = gate("other");
        assert!(Arc::ptr_eq(&first, &same));
        assert!(!Arc::ptr_eq(&first, &other));
    }
}
