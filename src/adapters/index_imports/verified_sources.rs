use super::super::{hash, read_source};
use super::{Failure, OperationContext, Project, sql};
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeMap;

// Bound retained allocations, not an arbitrary count of small importers. Global
// retry resolution can legitimately observe more than 64 files in one publication.
const MAX_RETAINED_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = super::super::MAX_ROWS;
const ENTRY_OVERHEAD: usize = 128; // Conservative per-entry bookkeeping allowance.
const ENTRY_LIMIT: Failure = ("SYNC_LIMIT", "Verified import source entry budget exceeded");
const BYTE_LIMIT: Failure = (
    "SYNC_LIMIT",
    "Verified import source retained-memory budget exceeded",
);

/// Source bytes accepted only when they still match the indexed publication.
#[derive(Default)]
pub(super) struct VerifiedSources {
    files: BTreeMap<String, Vec<u8>>,
    reserved: usize,
}

impl VerifiedSources {
    pub(super) fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    pub(super) fn len(&self) -> usize {
        self.files.len()
    }

    pub(super) fn bytes(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    pub(super) fn observe(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
    ) -> Result<bool, Failure> {
        let _timer = crate::adapters::index_profile::operation("verified_observe");
        context.check()?;
        if !self.files.contains_key(path) {
            crate::adapters::index_profile::count("verified_cache_misses", 1);
            let digest: Option<String> = {
                let _timer = crate::adapters::index_profile::operation("verified_index_lookup");
                let mut statement =
                    sql(db.prepare_cached("SELECT content_hash FROM files WHERE path=?1"))?;
                sql(statement.query_row([path], |r| r.get(0)).optional())?
            };
            if digest.is_none() {
                crate::adapters::index_profile::count("verified_not_indexed", 1);
                return Ok(false);
            }
            if self.files.len() >= MAX_ENTRIES {
                crate::adapters::index_profile::count("verified_entry_budget_exceeded", 1);
                return Err(ENTRY_LIMIT);
            }
            let remaining = MAX_RETAINED_BYTES.saturating_sub(self.reserved);
            let metadata_bytes = path.len().checked_add(ENTRY_OVERHEAD).ok_or(BYTE_LIMIT)?;
            if metadata_bytes > remaining {
                crate::adapters::index_profile::count("verified_byte_budget_exceeded", 1);
                return Err(BYTE_LIMIT);
            }
            let Some(bytes) = read_source(project, path, context)? else {
                return Ok(false);
            };
            // read_source is independently bounded to one source buffer. Account
            // for capacity, not just length, before retaining that buffer.
            let reservation = metadata_bytes
                .checked_add(bytes.capacity())
                .ok_or(BYTE_LIMIT)?;
            if reservation > remaining {
                crate::adapters::index_profile::count("verified_byte_budget_exceeded", 1);
                return Err(BYTE_LIMIT);
            }
            let current_hash = {
                let _timer = crate::adapters::index_profile::operation("verified_initial_hash");
                crate::adapters::index_profile::count("verified_hash_bytes", bytes.len() as u64);
                hash(&bytes)
            };
            if digest.as_deref() != Some(current_hash.as_str()) {
                crate::adapters::index_profile::count("verified_hash_mismatch", 1);
                return Ok(false);
            }
            self.reserved += reservation;
            self.files.insert(path.into(), bytes);
        } else {
            crate::adapters::index_profile::count("verified_cache_hits", 1);
        }
        Ok(true)
    }

    pub(super) fn recheck(
        &self,
        project: &Project,
        context: &OperationContext,
    ) -> Result<(), Failure> {
        for (path, bytes) in &self.files {
            crate::adapters::index_profile::count("verified_recheck_files", 1);
            crate::adapters::index_profile::count("verified_recheck_bytes", bytes.len() as u64);
            // Both buffers are already available: exact byte equality avoids two
            // redundant digests while retaining the fresh, bounded source read.
            let current = read_source(project, path, context)?;
            let _timer = crate::adapters::index_profile::operation("verified_compare");
            if current.as_deref() != Some(bytes.as_slice()) {
                return Err((
                    "STALE",
                    "Import resolution source changed; transaction rolled back",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "verified_sources_tests.rs"]
mod tests;
