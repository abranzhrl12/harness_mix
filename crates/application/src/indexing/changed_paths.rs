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
    let mut warnings = Vec::new();
    let mut warning_files = 0usize;
    for path in paths {
        let applied = context
            .check()
            .and_then(|()| port.apply_changed(path, context));
        match applied {
            Ok(Some(result)) => {
                processed += 1;
                if let Some(file_warnings) = result["warnings"].as_array() {
                    warning_files += result["warningFiles"].as_u64().unwrap_or(1) as usize;
                    warnings.extend(
                        file_warnings
                            .iter()
                            .take(32usize.saturating_sub(warnings.len()))
                            .cloned(),
                    );
                }
                changed += usize::from(result["changed"] == true);
                if result["sqlRefreshComplete"] == false {
                    let entry = json!({
                        "path":path,
                        "code":"SYNC_SQL_PARTIAL",
                        "message":"SQL dependents were only partially refreshed; see dependentFailures and dependentFilesCommitted",
                        "commitState":if result["dependentFilesCommitted"].as_array().is_some_and(|files| !files.is_empty()) {"partial"} else {"not_committed"},
                        "sqlRefresh":result,
                    });
                    failed += 1;
                    if failure.is_none() {
                        failure = Some(entry.clone());
                    }
                    if failures.len() < 32 {
                        failures.push(entry);
                    }
                }
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
        "selectedScopeApplied":failure.is_none() && warning_files==0,"sourceScopeApplied":failure.is_none(),"workspaceScanPerformed":false,
        "warnings":warnings,"warningFiles":warning_files,"warningReportLimit":32,"warningReportsOmitted":warning_files.saturating_sub(32),
        "globalFreshnessCertified":false,"workspaceTransactionAtomic":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockChangedPaths {
        result: Option<Value>,
    }

    impl ChangedPathsPort for MockChangedPaths {
        fn initialize(&mut self, _: &OperationContext) -> Result<(), Failure> {
            Ok(())
        }

        fn needs_inventory(&self, _: &BTreeSet<String>) -> Result<bool, Failure> {
            Ok(false)
        }

        fn sync_workspace(&mut self, _: &OperationContext) -> Result<Value, Failure> {
            unreachable!("these tests exercise incremental paths")
        }

        fn apply_changed(
            &mut self,
            _: &str,
            _: &OperationContext,
        ) -> Result<Option<Value>, Failure> {
            Ok(self.result.take())
        }
    }

    fn partial_result(committed: &[&str]) -> Value {
        json!({
            "changed": !committed.is_empty(),
            "sqlRefreshComplete": false,
            "dependentFilesCommitted": committed,
            "dependentFailures": [{"path":"Broken.cs","code":"SYNC_FILE","message":"fixture failure"}],
        })
    }

    fn run_partial(committed: &[&str]) -> Value {
        let mut port = MockChangedPaths {
            result: Some(partial_result(committed)),
        };
        sync_changed_paths(
            &mut port,
            &BTreeSet::from(["schema.sql".to_owned()]),
            &OperationContext::default(),
        )
        .unwrap()
    }

    #[test]
    fn sql_partial_preserves_committed_dependents_and_fails_batch_report() {
        let result = run_partial(&["Committed.cs"]);
        assert_eq!(result["changedFilesCommitted"], 1);
        assert_eq!(result["failedFiles"], 1);
        assert_eq!(result["failure"]["code"], "SYNC_SQL_PARTIAL");
        assert_eq!(result["failure"]["commitState"], "partial");
        assert_eq!(
            result["failure"]["sqlRefresh"]["dependentFilesCommitted"][0],
            "Committed.cs"
        );
        assert_eq!(result["selectedScopeApplied"], false);
    }

    #[test]
    fn sql_partial_without_confirmed_commit_reports_not_committed() {
        let result = run_partial(&[]);
        assert_eq!(result["changedFilesCommitted"], 0);
        assert_eq!(result["failure"]["code"], "SYNC_SQL_PARTIAL");
        assert_eq!(result["failure"]["commitState"], "not_committed");
    }

    #[test]
    fn incremental_publication_keeps_partial_graph_warnings_without_failing_sources() {
        let mut port = MockChangedPaths {
            result: Some(json!({"changed":true,
            "warningFiles":1,"warnings":[{"code":"INDEX_DERIVED_RELATIONS_INVALIDATED"}]})),
        };
        let report = sync_changed_paths(
            &mut port,
            &BTreeSet::from(["a.rs".to_owned()]),
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(report["sourceScopeApplied"], true);
        assert_eq!(report["selectedScopeApplied"], false);
        assert_eq!(report["warningFiles"], 1);
        assert_eq!(
            report["warnings"][0]["code"],
            "INDEX_DERIVED_RELATIONS_INVALIDATED"
        );
        assert!(report["failure"].is_null());
    }
}
