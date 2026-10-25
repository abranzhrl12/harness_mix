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
#[path = "freshness_tests.rs"]
mod tests;
