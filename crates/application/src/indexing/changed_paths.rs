//! Event-driven indexing policy and partial-commit reporting.
use crate::ports::indexing::ChangedPathsPort;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::collections::BTreeSet;
const LIMIT: Failure = (
    "SYNC_LIMIT",
    "Native publication exceeded its bounded graph budget",
);
const UNCERTAIN: Failure = (
    "SYNC_COMMIT_UNCERTAIN",
    "Index commit or rollback could not be confirmed; inspect the index before retrying",
);

pub fn sync_changed_paths<P: ChangedPathsPort>(
    port: &mut P,
    paths: &BTreeSet<String>,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if paths.len() > 1024 {
        return Err(LIMIT);
    }
    context.check()?;
    port.initialize(context)?;
    context.check()?;
    if port.needs_inventory(paths)? {
        return port.sync_workspace(context);
    }
    let mut changed = 0;
    let mut processed = 0;
    let mut skipped = 0;
    let mut failure = None;
    let mut failures = Vec::new();
    let mut failed = 0usize;
    let mut stopped_early = false;
    for path in paths {
        let applied = context
            .check()
            .and_then(|()| port.apply_changed(path, context));
        match applied {
            Ok(Some(result)) => {
                processed += 1;
                changed += usize::from(result["changed"] == true);
            }
            Ok(None) => skipped += 1,
            Err((code, message)) => {
                let entry = json!({"path":path,"code":code,"message":message,"commitState":if code==UNCERTAIN.0 {"unknown"}else{"not_committed"}});
                failed += 1;
                if failure.is_none() {
                    failure = Some(entry.clone());
                }
                if failures.len() < 32 {
                    failures.push(entry);
                }
                if super::workspace::systemic_failure(code)
                    || !port.file_failure_is_isolated()
                    || context.check().is_err()
                {
                    stopped_early = true;
                    break;
                }
            }
        }
    }
    Ok(
        json!({"mode":"sync_changed_paths","filesProcessed":processed,
        "changedFilesCommitted":changed,"skippedFiles":skipped,"failure":failure,
        "failedFiles":failed,"failures":failures,"failureReportLimit":32,"failureReportsOmitted":failed.saturating_sub(32),
        "nativeFilesRemaining":paths.len().saturating_sub(processed+skipped),
        "unattemptedFiles":paths.len().saturating_sub(processed+skipped+failed),"stoppedEarly":stopped_early,
        "selectedScopeApplied":failure.is_none(),"workspaceScanPerformed":false,
        "globalFreshnessCertified":false,"workspaceTransactionAtomic":false}),
    )
}
