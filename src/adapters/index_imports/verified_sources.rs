use super::super::{hash, read_source};
use super::{Failure, LIMIT, OperationContext, Project, sql};
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeMap;

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
        if !self.files.contains_key(path) {
            crate::adapters::index_profile::count("verified_cache_misses", 1);
            if self.files.len() >= 64 || self.reserved + 1024 * 1024 > 64 * 1024 * 1024 {
                return Err(LIMIT);
            }
            let Some(bytes) = read_source(project, path, context)? else {
                return Ok(false);
            };
            let digest: Option<String> = {
                let _timer = crate::adapters::index_profile::operation("verified_index_lookup");
                sql(db
                .query_row(
                    "SELECT content_hash FROM files WHERE path=?1",
                    [path],
                    |r| r.get(0),
                )
                .optional())?
            };
            let current_hash = {
                let _timer = crate::adapters::index_profile::operation("verified_initial_hash");
                crate::adapters::index_profile::count("verified_hash_bytes", bytes.len() as u64);
                hash(&bytes)
            };
            if digest.as_deref() != Some(current_hash.as_str()) {
                crate::adapters::index_profile::count(if digest.is_none() { "verified_not_indexed" } else { "verified_hash_mismatch" }, 1);
                return Ok(false);
            }
            self.reserved += bytes.len();
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
mod tests {
    use super::*;
    use std::fs::{self, FileTimes, OpenOptions};

    fn observed(root: &std::path::Path, bytes: &[u8]) -> (Project, VerifiedSources) {
        fs::write(root.join("source.rs"), bytes).unwrap();
        let project = Project::new(root).unwrap();
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE files(path TEXT PRIMARY KEY, content_hash TEXT);")
            .unwrap();
        db.execute("INSERT INTO files VALUES('source.rs',?1)", [hash(bytes)])
            .unwrap();
        let mut sources = VerifiedSources::default();
        assert!(
            sources
                .observe(&db, &project, &OperationContext::default(), "source.rs")
                .unwrap()
        );
        (project, sources)
    }

    #[test]
    fn recheck_accepts_identical_bytes_including_empty_and_non_utf8() {
        for bytes in [b"".as_slice(), b"fn main() {}\r\n", &[0xff, 0x00, 0xfe]] {
            let root = tempfile::tempdir().unwrap();
            let (project, sources) = observed(root.path(), bytes);
            sources
                .recheck(&project, &OperationContext::default())
                .unwrap();
        }
    }

    #[test]
    fn recheck_detects_same_size_change_even_when_timestamp_is_preserved() {
        let root = tempfile::tempdir().unwrap();
        let (project, sources) = observed(root.path(), b"alpha");
        let file = root.path().join("source.rs");
        let modified = fs::metadata(&file).unwrap().modified().unwrap();
        fs::write(&file, b"bravo").unwrap();
        OpenOptions::new()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(
            sources
                .recheck(&project, &OperationContext::default())
                .unwrap_err()
                .0,
            "STALE"
        );
    }

    #[test]
    fn recheck_distinguishes_missing_from_empty_and_preserves_cancellation() {
        let root = tempfile::tempdir().unwrap();
        let (project, sources) = observed(root.path(), b"");
        fs::remove_file(root.path().join("source.rs")).unwrap();
        assert_eq!(
            sources
                .recheck(&project, &OperationContext::default())
                .unwrap_err()
                .0,
            "STALE"
        );
        let cancelled = OperationContext::default();
        cancelled.cancel();
        assert_eq!(
            sources.recheck(&project, &cancelled).unwrap_err().0,
            "CANCELLED"
        );
        let expired = OperationContext::with_timeout(std::time::Duration::ZERO);
        assert_eq!(
            sources.recheck(&project, &expired).unwrap_err().0,
            "TIMEOUT"
        );
    }
}
