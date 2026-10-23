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
            context.check()?;
            let digest: Option<String> = {
                let _timer = crate::adapters::index_profile::operation("verified_index_lookup");
                let mut statement = sql(
                    db.prepare_cached("SELECT content_hash FROM files WHERE path=?1"),
                )?;
                sql(statement.query_row([path], |r| r.get(0)).optional())?
            };
            if digest.is_none() {
                crate::adapters::index_profile::count("verified_not_indexed", 1);
                return Ok(false);
            }
            let Some(bytes) = read_source(project, path, context)? else {
                return Ok(false);
            };
            let current_hash = {
                let _timer = crate::adapters::index_profile::operation("verified_initial_hash");
                crate::adapters::index_profile::count("verified_hash_bytes", bytes.len() as u64);
                hash(&bytes)
            };
            if digest.as_deref() != Some(current_hash.as_str()) {
                crate::adapters::index_profile::count("verified_hash_mismatch", 1);
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

    struct ProfileGuard;

    impl Drop for ProfileGuard {
        fn drop(&mut self) {
            crate::adapters::index_profile::finish();
        }
    }

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE files(path TEXT PRIMARY KEY, content_hash TEXT);")
            .unwrap();
        db
    }

    fn observed(root: &std::path::Path, bytes: &[u8]) -> (Project, VerifiedSources) {
        fs::write(root.join("source.rs"), bytes).unwrap();
        let project = Project::new(root).unwrap();
        let db = database();
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
    fn unindexed_candidates_skip_source_reads_and_are_not_cached_as_missing() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("existing.rs"), b"present but unindexed").unwrap();
        let project = Project::new(root.path()).unwrap();
        let db = database();
        let mut sources = VerifiedSources::default();

        crate::adapters::index_profile::begin(true);
        let _profile = ProfileGuard;
        crate::adapters::index_profile::stage("resolve_references");
        assert!(!sources
            .observe(&db, &project, &OperationContext::default(), "existing.rs")
            .unwrap());
        assert!(!sources
            .observe(&db, &project, &OperationContext::default(), "missing.rs")
            .unwrap());
        fs::write(root.path().join("missing.rs"), b"created during publication").unwrap();
        assert!(!sources
            .observe(&db, &project, &OperationContext::default(), "missing.rs")
            .unwrap());

        let metrics = crate::adapters::index_profile::metrics().unwrap();
        assert_eq!(metrics["operations"].get("source_read_resolution"), None);
        assert_eq!(metrics["sourceReads"].as_object().unwrap().len(), 0);
        assert_eq!(metrics["counters"]["verified_not_indexed"], 3);
    }

    #[test]
    fn indexed_missing_source_can_appear_and_empty_digest_is_not_absence() {
        let root = tempfile::tempdir().unwrap();
        let project = Project::new(root.path()).unwrap();
        let db = database();
        let bytes = b"created after initial missing observation";
        db.execute("INSERT INTO files VALUES('later.rs',?1)", [hash(bytes)])
            .unwrap();
        db.execute_batch("INSERT INTO files VALUES('empty-digest.rs','');")
            .unwrap();
        let mut sources = VerifiedSources::default();

        crate::adapters::index_profile::begin(true);
        let _profile = ProfileGuard;
        crate::adapters::index_profile::stage("resolve_references");
        assert!(!sources
            .observe(&db, &project, &OperationContext::default(), "later.rs")
            .unwrap());
        fs::write(root.path().join("later.rs"), bytes).unwrap();
        assert!(sources
            .observe(&db, &project, &OperationContext::default(), "later.rs")
            .unwrap());

        fs::write(root.path().join("empty-digest.rs"), b"has bytes").unwrap();
        assert!(!sources
            .observe(&db, &project, &OperationContext::default(), "empty-digest.rs")
            .unwrap());

        let metrics = crate::adapters::index_profile::metrics().unwrap();
        assert_eq!(metrics["sourceReads"]["later.rs"], 2);
        assert_eq!(metrics["sourceReads"]["empty-digest.rs"], 1);
        assert_eq!(metrics["counters"]["verified_hash_mismatch"], 1);
    }

    #[test]
    fn unindexed_fast_path_preserves_cancellation_and_indexed_paths_stay_guarded() {
        let root = tempfile::tempdir().unwrap();
        let project = Project::new(root.path()).unwrap();
        let db = database();
        let mut sources = VerifiedSources::default();

        let cancelled = OperationContext::default();
        cancelled.cancel();
        assert_eq!(
            sources
                .observe(&db, &project, &cancelled, "unindexed.rs")
                .unwrap_err()
                .0,
            "CANCELLED"
        );
        let expired = OperationContext::with_timeout(std::time::Duration::ZERO);
        assert_eq!(
            sources
                .observe(&db, &project, &expired, "unindexed.rs")
                .unwrap_err()
                .0,
            "TIMEOUT"
        );

        db.execute_batch("INSERT INTO files VALUES('../outside.rs','outside-hash');")
            .unwrap();
        assert_eq!(
            sources
                .observe(
                    &db,
                    &project,
                    &OperationContext::default(),
                    "../outside.rs"
                )
                .unwrap_err()
                .0,
            "PATH"
        );
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
    fn database_errors_are_not_mistaken_for_unindexed_candidates() {
        let root = tempfile::tempdir().unwrap();
        let project = Project::new(root.path()).unwrap();
        let db = Connection::open_in_memory().unwrap();
        let mut sources = VerifiedSources::default();
        // A broken index must fail even when the source is absent on disk.
        assert!(sources.observe(&db, &project, &OperationContext::default(),
            "missing.rs").is_err());
        assert_eq!(sources.len(), 0);
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
