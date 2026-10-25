    use super::{MAX_FILE_BYTES, certify, sha256_digest};
    use crate::adapters::{project::Project, symbol_graph::model::Candidate};
    use harness_domain::{execution::OperationContext, file_freshness::Verdict};
    use std::{
        fs::{self, FileTimes, OpenOptions},
        path::Path,
        time::{Duration, SystemTime},
    };

    fn candidate(indexed_hash: Option<String>) -> Candidate {
        Candidate {
            id: "symbol-1".into(),
            name: "symbol".into(),
            kind: "function".into(),
            qualified_name: "symbol".into(),
            file_path: "src.rs".into(),
            start_line: 1,
            end_line: 1,
            signature: "fn symbol()".into(),
            docstring: String::new(),
            language: "rust".into(),
            indexed_hash,
        }
    }

    fn write_file(root: &Path, bytes: &[u8], modified: SystemTime) {
        fs::write(root.join("src.rs"), bytes).unwrap();
        OpenOptions::new()
            .write(true)
            .open(root.join("src.rs"))
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
    }

    fn digest(bytes: &[u8]) -> String {
        sha256_digest(bytes).to_hex()
    }

    #[test]
    fn same_relative_path_in_different_projects_uses_each_projects_bytes() {
        let first_root = tempfile::tempdir().unwrap();
        let second_root = tempfile::tempdir().unwrap();
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        write_file(first_root.path(), b"alpha", modified);
        write_file(second_root.path(), b"bravo", modified);
        let first = Project::new(first_root.path()).unwrap();
        let second = Project::new(second_root.path()).unwrap();
        let indexed = Some(digest(b"alpha"));
        let candidate = candidate(indexed);

        assert_eq!(
            certify(&first, std::slice::from_ref(&candidate), &OperationContext::default())
                .get("src.rs"),
            Some(&Verdict::Fresh)
        );
        assert_eq!(
            certify(&second, &[candidate], &OperationContext::default()).get("src.rs"),
            Some(&Verdict::Stale)
        );
    }

    #[test]
    fn same_size_content_change_with_restored_timestamp_is_stale() {
        let root = tempfile::tempdir().unwrap();
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        write_file(root.path(), b"alpha", modified);
        let project = Project::new(root.path()).unwrap();
        let candidate = candidate(Some(digest(b"alpha")));
        let context = OperationContext::default();

        assert_eq!(
            certify(&project, std::slice::from_ref(&candidate), &context).get("src.rs"),
            Some(&Verdict::Fresh)
        );
        let original_modified = fs::metadata(root.path().join("src.rs"))
            .unwrap()
            .modified()
            .unwrap();
        write_file(root.path(), b"bravo", original_modified);
        assert_eq!(
            fs::metadata(root.path().join("src.rs"))
                .unwrap()
                .modified()
                .unwrap(),
            original_modified
        );

        assert_eq!(
            certify(&project, &[candidate], &context).get("src.rs"),
            Some(&Verdict::Stale)
        );
    }

    #[test]
    fn each_request_compares_its_indexed_hash() {
        let root = tempfile::tempdir().unwrap();
        write_file(
            root.path(),
            b"alpha",
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        );
        let project = Project::new(root.path()).unwrap();
        let context = OperationContext::default();

        assert_eq!(
            certify(&project, &[candidate(Some(digest(b"alpha")))], &context).get("src.rs"),
            Some(&Verdict::Fresh)
        );
        assert_eq!(
            certify(&project, &[candidate(Some(digest(b"other")))], &context).get("src.rs"),
            Some(&Verdict::Stale)
        );
    }

    #[test]
    fn missing_and_cancelled_reads_do_not_poison_later_certification() {
        let root = tempfile::tempdir().unwrap();
        let project = Project::new(root.path()).unwrap();
        let context = OperationContext::default();
        let missing = candidate(Some(digest(b"alpha")));
        assert_eq!(
            certify(&project, std::slice::from_ref(&missing), &context).get("src.rs"),
            Some(&Verdict::Missing)
        );

        write_file(
            root.path(),
            b"alpha",
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        );
        let cancelled = OperationContext::default();
        cancelled.cancel();
        assert_eq!(
            certify(&project, std::slice::from_ref(&missing), &cancelled).get("src.rs"),
            Some(&Verdict::Unknown)
        );
        assert_eq!(
            certify(&project, &[missing], &context).get("src.rs"),
            Some(&Verdict::Fresh)
        );
    }

    #[test]
    fn oversized_file_is_unknown_and_a_later_valid_file_is_rechecked() {
        let root = tempfile::tempdir().unwrap();
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        write_file(root.path(), &vec![b'x'; (MAX_FILE_BYTES + 1) as usize], modified);
        let project = Project::new(root.path()).unwrap();
        let candidate = candidate(Some(digest(b"alpha")));
        let context = OperationContext::default();

        assert_eq!(
            certify(&project, std::slice::from_ref(&candidate), &context).get("src.rs"),
            Some(&Verdict::Unknown)
        );
        write_file(root.path(), b"alpha", modified);
        assert_eq!(
            certify(&project, &[candidate], &context).get("src.rs"),
            Some(&Verdict::Fresh)
        );
    }

    #[test]
    fn summary_preserves_verdict_names_and_counts() {
        let verdicts = std::collections::BTreeMap::from([
            ("a.rs".to_owned(), Verdict::Fresh),
            ("b.rs".to_owned(), Verdict::Stale),
            ("c.rs".to_owned(), Verdict::Missing),
            ("d.rs".to_owned(), Verdict::Unknown),
        ]);

        assert_eq!(
            super::freshness_summary(&verdicts),
            serde_json::json!({"files":4,"fresh":1,"stale":1,"missing":1,"unknown":1})
        );
    }
