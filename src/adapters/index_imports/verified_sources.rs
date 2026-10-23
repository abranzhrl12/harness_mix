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
    fn many_small_sources_are_retained_and_the_last_source_is_rechecked() {
        let root = tempfile::tempdir().unwrap();
        let project = Project::new(root.path()).unwrap();
        let db = database();
        let mut sources = VerifiedSources::default();
        let context = OperationContext::default();
        for index in 0..128 {
            let path = format!("source-{index}.ts");
            fs::write(root.path().join(&path), b"export const x = 1;").unwrap();
            db.execute(
                "INSERT INTO files VALUES(?1,?2)",
                [&path, &hash(b"export const x = 1;")],
            )
            .unwrap();
            assert!(sources.observe(&db, &project, &context, &path).unwrap());
        }
        assert_eq!(sources.len(), 128);
        sources.recheck(&project, &context).unwrap();
        fs::write(root.path().join("source-127.ts"), b"export const x = 2;").unwrap();
        assert_eq!(sources.recheck(&project, &context).unwrap_err().0, "STALE");
    }

    #[test]
    fn unindexed_paths_still_skip_io_when_the_retained_budget_is_full() {
        let root = tempfile::tempdir().unwrap();
        let project = Project::new(root.path()).unwrap();
        let db = database();
        let mut sources = VerifiedSources {
            files: BTreeMap::new(),
            reserved: 64 * 1024 * 1024,
        };
        assert!(
            !sources
                .observe(&db, &project, &OperationContext::default(), "unindexed.ts")
                .unwrap()
        );
        db.execute("INSERT INTO files VALUES('indexed.ts',?1)", [hash(b"")])
            .unwrap();
        assert_eq!(
            sources
                .observe(&db, &project, &OperationContext::default(), "indexed.ts")
                .unwrap_err()
                .0,
            "SYNC_LIMIT"
        );
        assert!(sources.files.is_empty());
    }

    #[test]
    fn retained_budget_counts_capacity_and_rejected_admission_keeps_existing_sources() {
        let root = tempfile::tempdir().unwrap();
        let (project, mut sources) = observed(root.path(), b"snapshot");
        assert_eq!(
            sources.reserved,
            sources.files["source.rs"].capacity() + "source.rs".len() + ENTRY_OVERHEAD
        );
        let prior = sources.reserved;
        let db = database();
        assert!(
            sources
                .observe(&db, &project, &OperationContext::default(), "source.rs")
                .unwrap()
        );
        assert_eq!(sources.reserved, prior);
        db.execute("INSERT INTO files VALUES('extra.ts',?1)", [hash(b"x")])
            .unwrap();
        sources.reserved = MAX_RETAINED_BYTES - 1;
        assert_eq!(
            sources.observe(&db, &project, &OperationContext::default(), "extra.ts"),
            Err(BYTE_LIMIT)
        );
        assert_eq!(sources.len(), 1);
        assert_eq!(sources.bytes("source.rs"), Some(b"snapshot".as_slice()));
        let cancel = OperationContext::default();
        cancel.cancel();
        assert_eq!(
            sources
                .observe(&db, &project, &cancel, "source.rs")
                .unwrap_err()
                .0,
            "CANCELLED"
        );
    }

    #[test]
    fn entry_budget_still_bounds_empty_sources_without_charging_unindexed_paths() {
        let root = tempfile::tempdir().unwrap();
        let project = Project::new(root.path()).unwrap();
        let db = database();
        let mut sources = VerifiedSources {
            files: (0..MAX_ENTRIES)
                .map(|index| (format!("source-{index}.ts"), Vec::new()))
                .collect(),
            reserved: 0,
        };
        assert!(
            !sources
                .observe(&db, &project, &OperationContext::default(), "unindexed.ts")
                .unwrap()
        );
        db.execute("INSERT INTO files VALUES('extra.ts',?1)", [hash(b"")])
            .unwrap();
        assert_eq!(
            sources.observe(&db, &project, &OperationContext::default(), "extra.ts"),
            Err(ENTRY_LIMIT)
        );
        assert_eq!(sources.len(), MAX_ENTRIES);
    }

    #[test]
    fn mismatched_sources_do_not_consume_retained_memory() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("stale.ts"), b"current").unwrap();
        let project = Project::new(root.path()).unwrap();
        let db = database();
        db.execute("INSERT INTO files VALUES('stale.ts',?1)", [hash(b"old")])
            .unwrap();
        let mut sources = VerifiedSources::default();
        assert!(
            !sources
                .observe(&db, &project, &OperationContext::default(), "stale.ts")
                .unwrap()
        );
        assert_eq!(sources.reserved, 0);
        assert_eq!(sources.len(), 0);
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
        assert!(
            !sources
                .observe(&db, &project, &OperationContext::default(), "existing.rs")
                .unwrap()
        );
        assert!(
            !sources
                .observe(&db, &project, &OperationContext::default(), "missing.rs")
                .unwrap()
        );
        fs::write(
            root.path().join("missing.rs"),
            b"created during publication",
        )
        .unwrap();
        assert!(
            !sources
                .observe(&db, &project, &OperationContext::default(), "missing.rs")
                .unwrap()
        );

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
        assert!(
            !sources
                .observe(&db, &project, &OperationContext::default(), "later.rs")
                .unwrap()
        );
        fs::write(root.path().join("later.rs"), bytes).unwrap();
        assert!(
            sources
                .observe(&db, &project, &OperationContext::default(), "later.rs")
                .unwrap()
        );

        fs::write(root.path().join("empty-digest.rs"), b"has bytes").unwrap();
        assert!(
            !sources
                .observe(
                    &db,
                    &project,
                    &OperationContext::default(),
                    "empty-digest.rs"
                )
                .unwrap()
        );

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
                .observe(&db, &project, &OperationContext::default(), "../outside.rs")
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
        assert!(
            sources
                .observe(&db, &project, &OperationContext::default(), "missing.rs")
                .is_err()
        );
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
