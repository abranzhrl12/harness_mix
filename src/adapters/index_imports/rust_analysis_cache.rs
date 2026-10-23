use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

const CACHE_LIMIT: usize = 32 * 1024 * 1024;
const CACHE_ENTRY_OVERHEAD: usize = 256;
const CACHE_ENTRY_LIMIT: usize = 512;
const ANALYZER_VERSION: &str = "rust-facts-v1";

#[derive(Default, Clone)]
pub(in crate::adapters) struct RustFactsCache(Arc<Mutex<CacheState>>);

#[derive(Default)]
struct CacheState {
    entries: BTreeMap<(String, String), CacheEntry>,
    fifo: VecDeque<(String, String)>,
    reserved: usize,
}

struct CacheEntry {
    source: Arc<[u8]>,
    facts: Arc<super::super::rust::Facts>,
    reserved: usize,
}

impl RustFactsCache {
    pub(super) fn get(
        &self,
        source: &[u8],
        digest: &str,
    ) -> Option<Arc<super::super::rust::Facts>> {
        let state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        let entry = state.entries.get(&(ANALYZER_VERSION.into(), digest.into()))?;
        (entry.source.as_ref() == source).then(|| Arc::clone(&entry.facts))
    }

    pub(super) fn insert(
        &self,
        source: &[u8],
        digest: String,
        facts: Arc<super::super::rust::Facts>,
    ) {
        let source_reserved = source.len();
        let facts_reserved = facts.reserved();
        let reserved = source_reserved
            .saturating_add(facts_reserved)
            .saturating_add(CACHE_ENTRY_OVERHEAD);
        if reserved > CACHE_LIMIT || facts_reserved > 16 * 1024 * 1024 {
            return;
        }
        let key = (ANALYZER_VERSION.into(), digest);
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        if state.entries.contains_key(&key) {
            return;
        }
        while state.reserved.saturating_add(reserved) > CACHE_LIMIT
            || state.entries.len() >= CACHE_ENTRY_LIMIT
        {
            let Some(oldest) = state.fifo.pop_front() else {
                break;
            };
            if let Some(entry) = state.entries.remove(&oldest) {
                state.reserved = state.reserved.saturating_sub(entry.reserved);
            }
        }
        state.reserved += reserved;
        state.fifo.push_back(key.clone());
        state.entries.insert(
            key,
            CacheEntry {
                source: Arc::from(source),
                facts,
                reserved,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_bytes_guard_digest_matches() {
        let cache = RustFactsCache::default();
        let facts = Arc::new(super::super::super::rust::Facts::default());
        cache.insert(b"source one", "same-digest".into(), Arc::clone(&facts));
        assert!(Arc::ptr_eq(
            &cache.get(b"source one", "same-digest").unwrap(),
            &facts
        ));
        assert!(cache.get(b"source two", "same-digest").is_none());
    }

    #[test]
    fn entry_limit_evicts_oldest_deterministically() {
        let cache = RustFactsCache::default();
        for index in 0..=CACHE_ENTRY_LIMIT {
            cache.insert(&[index as u8], index.to_string(), Arc::new(super::super::super::rust::Facts::default()));
        }
        let state = cache.0.lock().unwrap();
        assert_eq!(state.entries.len(), CACHE_ENTRY_LIMIT);
        assert!(!state.entries.contains_key(&(ANALYZER_VERSION.into(), "0".into())));
        assert!(state.entries.contains_key(&(ANALYZER_VERSION.into(), CACHE_ENTRY_LIMIT.to_string())));
    }

    #[test]
    fn byte_limit_evicts_oldest_and_skips_oversized_entries() {
        let cache = RustFactsCache::default();
        let source = vec![7; CACHE_LIMIT / 3];
        for index in 0..3 {
            cache.insert(&source, index.to_string(), Arc::new(super::super::super::rust::Facts::default()));
        }
        {
            let state = cache.0.lock().unwrap();
            assert!(state.reserved <= CACHE_LIMIT);
            assert!(!state.entries.contains_key(&(ANALYZER_VERSION.into(), "0".into())));
            assert!(state.entries.contains_key(&(ANALYZER_VERSION.into(), "2".into())));
        }

        let too_large = vec![0; CACHE_LIMIT];
        cache.insert(&too_large, "oversized".into(), Arc::new(super::super::super::rust::Facts::default()));
        assert!(!cache
            .0
            .lock()
            .unwrap()
            .entries
            .contains_key(&(ANALYZER_VERSION.into(), "oversized".into())));
    }
}
