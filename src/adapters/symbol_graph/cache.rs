use super::model::Verdict;
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::UNIX_EPOCH;

/// Cached verdict per file, keyed by the observed size and modification time.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Stamp {
    modified_nanos: u128,
    size: u64,
}

/// Verdicts cached per file: the observed stamp, the indexed hash and the verdict.
type VerdictCache = Mutex<BTreeMap<String, (Stamp, String, Verdict)>>;

fn cache() -> &'static VerdictCache {
    static CACHE: OnceLock<VerdictCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn lock() -> MutexGuard<'static, BTreeMap<String, (Stamp, String, Verdict)>> {
    cache().lock().unwrap_or_else(|error| error.into_inner())
}

pub(super) fn stamp_of(metadata: &std::fs::Metadata) -> Stamp {
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_nanos());
    Stamp {
        modified_nanos,
        size: metadata.len(),
    }
}
