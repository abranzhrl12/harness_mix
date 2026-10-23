use super::super::project::Project;
use super::model::{Candidate, Verdict};
use harness_domain::{execution::OperationContext, hashing::sha256_digest};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Certify candidate files against the working tree.
///
/// Current bytes are read and hashed for every request so metadata cannot
/// certify stale content. Duplicate candidate paths are deduplicated per call.
pub(in crate::adapters) fn certify(
    project: &Project,
    candidates: &[Candidate],
    context: &OperationContext,
) -> BTreeMap<String, Verdict> {
    let mut verdicts = BTreeMap::new();
    for candidate in candidates {
        if verdicts.contains_key(&candidate.file_path) {
            continue;
        }
        if let Err((code, _)) = context.check()
            && (code == "CANCELLED" || code == "TIMEOUT")
        {
            verdicts.insert(candidate.file_path.clone(), Verdict::Unknown);
            break;
        }
        let absolute = match project.target_optional(&candidate.file_path) {
            Ok(path) => path,
            Err(_) => {
                verdicts.insert(candidate.file_path.clone(), Verdict::Unknown);
                continue;
            }
        };
        let metadata = match std::fs::metadata(&absolute) {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => {
                verdicts.insert(candidate.file_path.clone(), Verdict::Missing);
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                verdicts.insert(candidate.file_path.clone(), Verdict::Missing);
                continue;
            }
            Err(_) => {
                verdicts.insert(candidate.file_path.clone(), Verdict::Unknown);
                continue;
            }
        };
        if metadata.len() > MAX_FILE_BYTES {
            verdicts.insert(candidate.file_path.clone(), Verdict::Unknown);
            continue;
        }
        let verdict = match project.bytes_with_limit(&candidate.file_path, context, MAX_FILE_BYTES)
        {
            Ok(bytes) => Verdict::from_indexed_hash(
                candidate.indexed_hash.as_deref(),
                &sha256_digest(&bytes),
            ),
            Err(_) => Verdict::Unknown,
        };
        verdicts.insert(candidate.file_path.clone(), verdict);
    }
    verdicts
}

/// Summary of the certification phase for the response envelope.
pub(in crate::adapters) fn freshness_summary(verdicts: &BTreeMap<String, Verdict>) -> Value {
    let mut summary = json!({"files":0,"fresh":0,"stale":0,"missing":0,"unknown":0});
    for verdict in verdicts.values() {
        summary["files"] = json!(summary["files"].as_u64().unwrap_or(0) + 1);
        summary[verdict.as_str()] = json!(summary[verdict.as_str()].as_u64().unwrap_or(0) + 1);
    }
    summary
}

#[cfg(test)]
mod tests {
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
}
