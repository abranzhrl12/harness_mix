    use super::*;
    use harness_domain::hash;
    use std::{fs, sync::Arc};

    fn setup(bytes: &[u8]) -> (tempfile::TempDir, Project, Connection) {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("lib.rs"), bytes).unwrap();
        let project = Project::new(root.path()).unwrap();
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE files(path TEXT PRIMARY KEY, content_hash TEXT);")
            .unwrap();
        db.execute("INSERT INTO files VALUES('lib.rs', ?1)", [hash(bytes)])
            .unwrap();
        (root, project, db)
    }

    fn analyze(
        resolution: &mut RustResolution,
        project: &Project,
        db: &Connection,
        context: &OperationContext,
    ) -> Result<bool, Failure> {
        resolution.analyzed(
            &mut VerifiedSources::default(),
            db,
            project,
            context,
            "lib.rs",
        )
    }

    #[test]
    fn reuses_only_verified_identical_bytes_across_publications() {
        let alpha = b"pub fn alpha() {}";
        let beta = b"pub fn beta() {}";
        let (_root, project, db) = setup(alpha);
        let cache = RustFactsCache::default();

        let mut first = RustResolution::with_cache(cache.clone());
        assert!(analyze(&mut first, &project, &db, &OperationContext::default()).unwrap());
        let first_facts = Arc::clone(&first.rust["lib.rs"]);

        let mut second = RustResolution::with_cache(cache.clone());
        assert!(analyze(&mut second, &project, &db, &OperationContext::default()).unwrap());
        assert!(Arc::ptr_eq(&first_facts, &second.rust["lib.rs"]));

        fs::write(_root.path().join("lib.rs"), beta).unwrap();
        let mut stale_index = RustResolution::with_cache(cache.clone());
        assert!(!analyze(&mut stale_index, &project, &db, &OperationContext::default()).unwrap());
        db.execute("UPDATE files SET content_hash = ?1 WHERE path='lib.rs'", [hash(beta)])
            .unwrap();
        let mut reindexed = RustResolution::with_cache(cache.clone());
        assert!(analyze(&mut reindexed, &project, &db, &OperationContext::default()).unwrap());
        assert!(reindexed.rust["lib.rs"].functions.contains_key("beta"));
        assert!(!reindexed.rust["lib.rs"].functions.contains_key("alpha"));

        fs::remove_file(_root.path().join("lib.rs")).unwrap();
        let mut missing = RustResolution::with_cache(cache);
        assert!(!analyze(&mut missing, &project, &db, &OperationContext::default()).unwrap());
    }

    #[test]
    fn cache_cannot_shortcut_cancellation_or_timeout() {
        let source = b"pub fn cached() {}";
        let (_root, project, db) = setup(source);
        let cache = RustFactsCache::default();
        let mut initial = RustResolution::with_cache(cache.clone());
        assert!(analyze(&mut initial, &project, &db, &OperationContext::default()).unwrap());

        let cancelled = OperationContext::default();
        cancelled.cancel();
        let mut next = RustResolution::with_cache(cache.clone());
        assert_eq!(analyze(&mut next, &project, &db, &cancelled).unwrap_err().0, "CANCELLED");

        let expired = OperationContext::with_timeout(std::time::Duration::ZERO);
        let mut next = RustResolution::with_cache(cache);
        assert_eq!(analyze(&mut next, &project, &db, &expired).unwrap_err().0, "TIMEOUT");
    }

    #[test]
    fn intra_publication_facts_hit_still_checks_cancellation() {
        let source = b"pub fn cached() {}";
        let (_root, project, db) = setup(source);
        let mut resolution = RustResolution::default();
        let mut sources = VerifiedSources::default();
        assert!(resolution
            .analyzed(&mut sources, &db, &project, &OperationContext::default(), "lib.rs")
            .unwrap());
        let cancelled = OperationContext::default();
        cancelled.cancel();
        assert_eq!(
            resolution
                .analyzed(&mut sources, &db, &project, &cancelled, "lib.rs")
                .unwrap_err()
                .0,
            "CANCELLED"
        );
    }

    #[test]
    fn per_publication_budget_keeps_only_empty_fallback_facts() {
        let source = b"pub fn cached() {}";
        let (_root, project, db) = setup(source);
        let cache = RustFactsCache::default();
        let mut initial = RustResolution::with_cache(cache.clone());
        assert!(analyze(&mut initial, &project, &db, &OperationContext::default()).unwrap());

        let mut over_budget = RustResolution::with_cache(cache);
        over_budget.rust_reserved = 16 * 1024 * 1024;
        assert!(analyze(&mut over_budget, &project, &db, &OperationContext::default()).unwrap());
        assert!(over_budget.rust["lib.rs"].functions.is_empty());
        assert!(over_budget.rust["lib.rs"].modules.is_empty());
        assert_eq!(over_budget.rust_reserved, 16 * 1024 * 1024);
    }

    #[test]
    fn shared_hit_still_rechecks_dependency_bytes_before_commit() {
        let source = b"pub fn alpha() {}";
        let (root, project, db) = setup(source);
        let cache = RustFactsCache::default();
        let mut first = RustResolution::with_cache(cache.clone());
        assert!(analyze(&mut first, &project, &db, &OperationContext::default()).unwrap());
        let mut second = RustResolution::with_cache(cache);
        let mut observed = VerifiedSources::default();
        assert!(second.analyzed(&mut observed, &db, &project,
            &OperationContext::default(), "lib.rs").unwrap());
        assert!(Arc::ptr_eq(&first.rust["lib.rs"], &second.rust["lib.rs"]));
        observed.recheck(&project, &OperationContext::default()).unwrap();
        let path = root.path().join("lib.rs");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, b"pub fn bravo() {}").unwrap();
        fs::OpenOptions::new().write(true).open(&path).unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified)).unwrap();
        assert_eq!(observed.recheck(&project, &OperationContext::default()).unwrap_err().0, "STALE");
    }
