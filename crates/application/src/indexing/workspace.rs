//! Workspace use case: bounded scheduling, error policy and partial-commit reporting.
use crate::ports::indexing::WorkspacePort;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::collections::BTreeSet;
const INDEX: Failure = (
    "SYNC_INDEX",
    "CodeGraph index incompatible, busy or invalid; no raw database diagnostics returned",
);
const UNCERTAIN: Failure = (
    "SYNC_COMMIT_UNCERTAIN",
    "Index commit or rollback could not be confirmed; inspect the index before retrying",
);

pub fn sync_workspace<P: WorkspacePort>(
    port: &mut P,
    context: &OperationContext,
    max_files: usize,
    rebuild: bool,
) -> Result<Value, Failure> {
    let operation_started = std::time::Instant::now();
    context.check()?;
    let scope = port.load_scope(context)?;
    context.check()?;
    let planned = port.plan(context)?;
    context.check()?;
    if planned["scope"] != scope {
        return Err(("STALE", "Scope changed during planning"));
    }
    let unchecked = planned["plan"]["unchecked"].as_array().ok_or(INDEX)?.len();
    let mut paths = BTreeSet::new();
    let mut unsupported = 0;
    for key in ["added", "modified", "retryExtraction", "removed"] {
        for path in planned["plan"][key].as_array().ok_or(INDEX)? {
            let path = path.as_str().ok_or(INDEX)?;
            if port.language(path).is_some() {
                paths.insert(path.to_owned());
            } else {
                unsupported += 1;
            }
        }
    }
    if rebuild {
        for path in planned["plan"]["unchanged"].as_array().ok_or(INDEX)? {
            let path = path.as_str().ok_or(INDEX)?;
            if port.language(path).is_some() {
                paths.insert(path.into());
            } else {
                unsupported += 1;
            }
        }
    }
    // A selected config can change without a source edit. Revisit one unchanged
    // JS/TS source so the transaction can invalidate/re-resolve import edges.
    if paths.is_empty()
        && let Some(path) = planned["plan"]["unchanged"]
            .as_array()
            .ok_or(INDEX)?
            .iter()
            .filter_map(Value::as_str)
            .find(|p| {
                matches!(
                    port.language(p),
                    Some("typescript" | "tsx" | "javascript" | "jsx")
                )
            })
    {
        paths.insert(path.into());
    }
    let total = paths.len();
    let planning_micros = operation_started.elapsed().as_micros();
    let started = std::time::Instant::now();
    context.report_progress("indexing", 0, Some(total), None);
    let mut results = Vec::new();
    let mut processed = 0;
    let mut changed = 0;
    let mut provider_dependent = 0;
    let mut failed = 0;
    let mut failure = None;
    let mut failures = Vec::new();
    let mut skipped_reports = Vec::new();
    let worker_count = port.workers().clamp(1, 4);
    let selected: Vec<_> = paths.iter().take(max_files).collect();
    let mut prepared = std::collections::VecDeque::new();
    let mut preparation_micros = 0u128;
    let mut phase_totals = std::collections::BTreeMap::<String, u64>::new();
    let mut reference_attempts = 0u64;
    let mut reference_resolved = 0u64;
    let mut nodes_written = 0u64;
    let mut profile_totals = super::profile::Totals::default();
    for (position, path) in selected.iter().copied().enumerate() {
        if prepared.is_empty() && worker_count > 1 && context.check().is_ok() {
            let end = (position + worker_count).min(selected.len());
            let preparation_started = std::time::Instant::now();
            let wave = port.prepare(&selected[position..end], context);
            if wave.len() == end - position {
                prepared.extend(wave);
            } else {
                // An optional accelerator may fail; never shift another file's
                // preparation into this slot when an implementation is partial.
                prepared.extend((position..end).map(|_| None));
            }
            preparation_micros += preparation_started.elapsed().as_micros();
        }
        let preparation = prepared.pop_front().flatten();
        context.report_progress("indexing", processed, Some(total), Some(path));
        let file_started = std::time::Instant::now();
        // Bound each file without extending the owner's deadline or losing cancellation.
        let file_context = context.with_execution_timeout(Some(std::time::Duration::from_secs(60)));
        let applied = context
            .check()
            .and_then(|()| port.apply(path, preparation, rebuild, &file_context));
        match applied {
            Ok(mut result) => {
                processed += 1;
                profile_totals.record(&result);
                if let Some(phases) = result["phaseTimingsMicros"].as_object() {
                    for (name, value) in phases {
                        *phase_totals.entry(name.clone()).or_default() +=
                            value.as_u64().unwrap_or(0);
                    }
                    reference_attempts += result["referencesAttempted"].as_u64().unwrap_or(0);
                    reference_resolved += result["referencesResolved"].as_u64().unwrap_or(0);
                    nodes_written += result["nodesWritten"].as_u64().unwrap_or(0);
                }
                changed += usize::from(result["changed"] == true);
                if results.len() < 32 {
                    result["durationMs"] = json!(file_started.elapsed().as_millis());
                    results.push(result);
                }
                context.report_progress("indexing", processed, Some(total), Some(path));
            }
            Err((code, message)) => {
                let mut entry = json!({"path":path,"code":code,"message":message,"deadlineScope":"file_within_workspace","fileTimeoutSeconds":60,"commitState":if code==UNCERTAIN.0 {"unknown"}else{"not_committed"}});
                if let Some(diagnostic) = port.diagnostic(code, message) {
                    entry["diagnostic"] = diagnostic;
                }
                // Provider-dependent files (framework or synthesized graph data
                // the native publisher cannot reproduce) are a documented
                // limitation: they are reported and skipped.
                if code == "SYNC_UNSUPPORTED_RELATION" || code == "SYNC_LEGACY_REFERENCE" {
                    provider_dependent += 1;
                    unsupported += 1;
                    entry["skipped"] = json!(true);
                    skipped_reports.push(entry.clone());
                    if results.len() < 32 {
                        entry["skipped"] = json!(true);
                        results.push(entry);
                    }
                    continue;
                }
                // A per-file failure does not stop the rest of the workspace:
                // the first failure is reported and the remaining files are
                // still published. Systemic failures still stop the run.
                if failure.is_none() {
                    failure = Some(entry.clone());
                }
                failures.push(entry.clone());
                if results.len() < 32 {
                    results.push(entry);
                }
                failed += 1;
                if matches!(
                    code,
                    "SYNC_SCHEMA"
                        | "SYNC_INDEX_BUSY"
                        | "SYNC_INTEGRITY"
                        | "SYNC_COMMIT_UNCERTAIN"
                        | "SYNC_LIMIT"
                        | "LIMIT"
                        | "PATH"
                ) || context.check().is_err()
                {
                    break;
                }
                continue;
            }
        }
    }
    context.report_progress("finalizing", processed, Some(total), None);
    let finalization_started = std::time::Instant::now();
    if failure.is_none()
        && let Err((code, message)) = port.recheck(context).and_then(|()| context.check())
    {
        let entry = json!({"path":"codegraph.json","code":code,"message":message,"stage":"final_scope_recheck","commitState":"not_committed"});
        failures.push(entry.clone());
        failure = Some(entry);
    }
    let native_complete = failure.is_none() && processed == total;
    let finalization_micros = finalization_started.elapsed().as_micros();
    let mut report = json!({"mode":"sync_native_subset","rebuild":rebuild,"files":results,"failure":failure,"scope":scope,
        "changedFilesCommitted":changed,"nativeCandidates":total,"nativeFilesRemaining":total.saturating_sub(processed).saturating_sub(provider_dependent),
        "failures":failures,"extractionWorkers":worker_count,"databaseWriters":1,
        "skippedFileReports":skipped_reports,"inventoryDiagnostics":planned["inventory"]["fileDiagnostics"],"uncheckedFilePaths":planned["plan"]["unchecked"],
        "indexingDurationMs":started.elapsed().as_millis(),
        "durationMs":operation_started.elapsed().as_millis(),
        "filesProcessed":processed,"fileReportsOmitted":(processed+failed+provider_dependent).saturating_sub(results.len()),"fileReportLimit":32,
        "maxFiles":max_files,"selectedScopeApplied":native_complete,"workspaceTransactionAtomic":false,
        "unsupportedChangedFiles":unsupported,"uncheckedFiles":unchecked,"providerDependentFiles":provider_dependent,"skippedFiles":provider_dependent,"failedFiles":failed,
        "restrictedIndexedFiles":planned["inventory"]["restrictedIndexedFiles"],
        "restrictedIndexedReasons":planned["inventory"]["restrictedIndexedReasons"],
        "discoveryComplete":planned["inventory"]["discoveryComplete"],
        "synchronized":false,"globalFreshnessCertified":false,"fullProviderScope":false,
        "resolutionPolicy":"native_exact_name_and_relative_named_import_subset"});
    if !phase_totals.is_empty() {
        report["planningDurationMicros"] = json!(planning_micros);
        report["finalizationDurationMicros"] = json!(finalization_micros);
        report["phaseTimingsMicros"] = json!(phase_totals);
        report["preparationDurationMicros"] = json!(preparation_micros);
        report["totalReferencesAttempted"] = json!(reference_attempts);
        report["totalReferencesResolved"] = json!(reference_resolved);
        report["totalNodesWritten"] = json!(nodes_written);
    }
    profile_totals.write(&mut report);
    Ok(report)
}
