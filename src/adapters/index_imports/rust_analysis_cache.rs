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
#[path = "rust_analysis_cache_tests.rs"]
mod tests;
