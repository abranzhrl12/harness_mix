use super::CONTRACT;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};

pub(in crate::adapters) fn preview(
    project: &super::super::project::Project,
    plan: &super::super::index_sync::Plan,
    observations: &std::collections::BTreeMap<String, super::super::index_sync::Observation>,
    context: &OperationContext,
) -> Result<Value, Failure> {
    use std::time::{Duration, Instant};
    let started = Instant::now();
    let candidates = plan
        .added
        .iter()
        .chain(&plan.modified)
        .chain(&plan.retry_extraction)
        .filter(|path| super::orchestration::language(path).is_some())
        .collect::<Vec<_>>();
    let mut files = Vec::new();
    for path in candidates.iter().take(4) {
        context.check()?;
        if started.elapsed() >= Duration::from_secs(5) {
            break;
        }
        let run = || {
            let validated = project.target(path)?;
            #[cfg(windows)]
            let _pins = super::super::edit_commit::pin_directory(
                project,
                validated
                    .parent()
                    .ok_or(("PATH", "Missing source parent"))?,
            )?;
            #[cfg(not(windows))]
            let _ = validated;
            let bytes = project.bytes_with_limit(
                path,
                context,
                super::super::index_sync::MAX_SOURCE_BYTES as u64,
            )?;
            let digest = super::super::index_sync::source_hash(&bytes)?;
            if !matches!(observations.get(*path), Some(super::super::index_sync::Observation::Content(observed)) if *observed == digest)
            {
                return Err(("STALE", "Source changed after inventory"));
            }
            let source =
                std::str::from_utf8(&bytes).map_err(|_| ("ENCODING", "Source is not UTF-8"))?;
            let extracted = super::orchestration::extract(path, source, context)?;
            let mut result = json!({"path":path,"sourceHash":digest,"status":"extracted_not_published",
                "nodes":extracted["nodes"].as_array().ok_or(CONTRACT)?.len(),
                "edges":extracted["edges"].as_array().ok_or(CONTRACT)?.len(),
                "unresolvedReferences":extracted["unresolvedReferences"].as_array().ok_or(CONTRACT)?.len()});
            if let Some(coverage) = extracted.get("coverage") {
                result["extractionCoverage"] = coverage.clone();
            }
            Ok(result)
        };
        match run() {
            Ok(value) => files.push(value),
            Err(failure @ ("CANCELLED" | "TIMEOUT", _)) => return Err(failure),
            Err((code, _)) => files.push(json!({"path":path,"status":"deferred","code":code})),
        }
    }
    Ok(
        json!({"provider":"codegraph-v1.6.2-native-subset","files":files,
        "pendingFiles":candidates.len().saturating_sub(files.len()),"maxFiles":4,
        "relationsResolved":false,"published":false,"sourceSnapshotAtomic":false}),
    )
}
