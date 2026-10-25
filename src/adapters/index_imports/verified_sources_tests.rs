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
