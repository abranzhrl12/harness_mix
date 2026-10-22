//! Atomic, explicit-file CodeGraph publication. No migrations or source writes.
use super::{index_extract, index_inventory, index_sync, project::Project};
use harness_domain::{Failure, execution::OperationContext, hash};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path, time::Duration};

const INDEX: Failure = (
    "SYNC_INDEX",
    "CodeGraph index incompatible, busy or invalid; no raw database diagnostics returned",
);
const INDEX_BUSY: Failure = (
    "SYNC_INDEX_BUSY",
    "CodeGraph index is busy or locked; close competing index clients and retry",
);
const LIMIT: Failure = (
    "SYNC_LIMIT",
    "Native publication exceeded its bounded graph budget",
);
const UNCERTAIN: Failure = (
    "SYNC_COMMIT_UNCERTAIN",
    "Index commit or rollback could not be confirmed; inspect the index before retrying",
);
const MAX_ROWS: usize = 20_000;

#[path = "index_store/sync_diagnostic.rs"]
mod sync_diagnostic;
#[path = "index_store/validation.rs"]
mod validation;
use self::validation::validate_schema;

#[path = "index_store/repository.rs"]
mod repository;
use self::repository::insert_ref;

#[path = "index_store/publication.rs"]
mod publication;
use self::publication::publish;

#[path = "index_store/resolution.rs"]
mod resolution;

#[cfg(test)]
#[path = "index_store/tests.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "index_store/rust_ra_tests.rs"]
mod rust_ra_tests;

#[path = "index_candidates.rs"]
mod candidates;
#[path = "index_razor_invalidation.rs"]
mod razor_invalidation;

#[cfg(windows)]
#[path = "index_writer.rs"]
mod writer;

#[path = "index_batches.rs"]
mod batches;

#[path = "index_init.rs"]
mod initialization;
pub use initialization::initialize;

#[path = "index_scope.rs"]
pub(crate) mod scope;

#[path = "index_imports.rs"]
mod imports;
#[path = "index_paths.rs"]
mod paths;
#[path = "index_razor_receivers.rs"]
mod razor_receivers;
#[path = "index_rust.rs"]
mod rust;
#[path = "index_tsconfig.rs"]
pub(super) mod tsconfig;

#[path = "index_semantic_policy.rs"]
mod semantic_policy;

#[path = "index_store/parallel.rs"]
mod parallel;

/// Apply a bounded native subset of the current plan. Transactions are per
/// file, NOT per workspace. Once any file commits, subsequent errors/cancellation
/// are returned in the JSON report alongside committed paths, never as a bare
/// error that could imply the earlier files were rolled back.
pub fn sync_workspace(root: &Path, context: &OperationContext) -> Result<Value, Failure> {
    sync_workspace_with_limit(root, context, 32, false)
}

/// Drain the bounded inventory rather than requiring one CLI call per 32 files.
/// Keep at most 32 detailed results; aggregate every confirmed commit separately.
pub fn sync_workspace_all(root: &Path, context: &OperationContext) -> Result<Value, Failure> {
    sync_workspace_with_limit(root, context, MAX_ROWS, false)
}

/// Re-extract unchanged files too, preserving the existing DB and per-file rollback.
/// This is a native-profile rebuild, not a schema migration or global transaction.
pub fn rebuild_workspace(root: &Path, context: &OperationContext) -> Result<Value, Failure> {
    sync_workspace_with_limit(root, context, MAX_ROWS, true)
}

/// Event-driven incremental publication: no workspace inventory, one connection
/// per bounded batch. Existing tracked files retain the full sync ignore policy.
pub fn sync_changed_paths(
    root: &Path,
    paths: &BTreeSet<String>,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if paths.len() > 1024 {
        return Err(LIMIT);
    }
    let project = Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?;
    let scope = scope::Snapshot::load(&project, context)?;
    let mut writer = Some(writer::Writer::open(&project, context)?);
    validate_schema(&writer.as_ref().ok_or(INDEX)?.db)?;
    if project.root.join(".git").is_file() {
        for path in paths {
            let tracked: bool = sql(writer.as_ref().ok_or(INDEX)?.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM files WHERE path=?1)",
                [path],
                |r| r.get(0),
            ))?;
            if !tracked && path.split('/').any(index_inventory::excluded) {
                continue;
            }
            if !tracked {
                drop(writer);
                return sync_workspace_all(root, context);
            }
        }
    }
    let mut changed = 0;
    let mut processed = 0;
    let mut skipped = 0;
    let mut failure = None;
    for path in paths {
        let applied = (|| {
            context.check()?;
            project.target_optional(path)?;
            scope.recheck(&project, context)?;
            if index_extract::language(path).is_none() || index_inventory::restricted(path) {
                return Ok(None);
            }
            let tracked: bool = sql(writer.as_ref().ok_or(INDEX)?.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM files WHERE path=?1)",
                [path],
                |r| r.get(0),
            ))?;
            if !tracked && path.split('/').any(index_inventory::excluded) {
                return Ok(None);
            }
            if !tracked && !scope.included(path) && ignored_new_source(&project, path, context)? {
                return Ok(None);
            }
            sync_file_config_reuse(root, path, None, context, || {}, false, &mut writer).map(Some)
        })();
        match applied {
            Ok(Some(result)) => {
                processed += 1;
                changed += usize::from(result["changed"] == true);
            }
            Ok(None) => skipped += 1,
            Err((code, message)) => {
                failure = Some(
                    json!({"path":path,"code":code,"message":message,"commitState":if code==UNCERTAIN.0 {"unknown"}else{"not_committed"}}),
                );
                break;
            }
        }
    }
    Ok(
        json!({"mode":"sync_changed_paths","filesProcessed":processed,
        "changedFilesCommitted":changed,"skippedFiles":skipped,"failure":failure,
        "selectedScopeApplied":failure.is_none(),"workspaceScanPerformed":false,
        "globalFreshnessCertified":false,"workspaceTransactionAtomic":false}),
    )
}

fn ignored_new_source(
    project: &Project,
    path: &str,
    context: &OperationContext,
) -> Result<bool, Failure> {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(&project.root);
    if project.root.join(".git").is_dir() {
        let exclude = project.target_optional(".git/info/exclude")?;
        if exclude.exists() {
            let bytes = project.bytes_with_limit(".git/info/exclude", context, 128 * 1024)?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| ("SYNC_IGNORE", "Invalid Git exclude file"))?;
            for line in text.lines() {
                builder
                    .add_line(Some(project.root.join(".gitignore")), line)
                    .map_err(|_| ("SYNC_IGNORE", "Invalid Git exclude pattern"))?;
            }
        }
    }
    // Same local .gitignore hierarchy as discovery, without walking or hashing
    // unrelated sources. Config includes deliberately override discovery ignores.
    let mut directory = String::new();
    let components: Vec<_> = path.split('/').collect();
    for depth in 0..components.len() {
        if depth > 0 {
            directory.push_str(components[depth - 1]);
            directory.push('/');
            // A child's .gitignore cannot resurrect a pruned parent directory.
            let parents = builder
                .build()
                .map_err(|_| ("SYNC_IGNORE", "Invalid ignore rules"))?;
            if parents
                .matched_path_or_any_parents(project.root.join(&directory), true)
                .is_ignore()
            {
                return Ok(true);
            }
        }
        let ignore_path = format!("{directory}.gitignore");
        let target = project.target_optional(&ignore_path)?;
        match std::fs::symlink_metadata(&target) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(("SYNC_IGNORE", "Ignore file unavailable")),
            Ok(_) => {}
        }
        let bytes = project.bytes_with_limit(&ignore_path, context, 128 * 1024)?;
        let text =
            std::str::from_utf8(&bytes).map_err(|_| ("SYNC_IGNORE", "Invalid ignore file"))?;
        for line in text.lines() {
            builder
                .add_line(Some(target.clone()), line)
                .map_err(|_| ("SYNC_IGNORE", "Invalid ignore pattern"))?;
        }
    }
    let rules = builder
        .build()
        .map_err(|_| ("SYNC_IGNORE", "Invalid ignore rules"))?;
    Ok(rules
        .matched_path_or_any_parents(project.root.join(path), false)
        .is_ignore())
}

fn sync_workspace_with_limit(
    root: &Path,
    context: &OperationContext,
    max_files: usize,
    rebuild: bool,
) -> Result<Value, Failure> {
    let operation_started = std::time::Instant::now();
    context.check()?;
    let project = Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?;
    let database = project.root.join(".codegraph/codegraph.db");
    sync_diagnostic::begin("<workspace-plan>", &database);
    sync_diagnostic::stage("load_workspace_scope");
    let scope = scope::Snapshot::load(&project, context)?;
    sync_diagnostic::stage("plan_workspace");
    let planned = index_sync::plan_workspace_for_sync(root, context)?;
    if planned["scope"] != scope.stamp() {
        return Err(("STALE", "Scope changed during planning"));
    }
    let unchecked = planned["plan"]["unchecked"].as_array().ok_or(INDEX)?.len();
    let mut paths = BTreeSet::new();
    let mut unsupported = 0;
    for key in ["added", "modified", "retryExtraction", "removed"] {
        for path in planned["plan"][key].as_array().ok_or(INDEX)? {
            let path = path.as_str().ok_or(INDEX)?;
            if index_extract::language(path).is_some() {
                paths.insert(path.to_owned());
            } else {
                unsupported += 1;
            }
        }
    }
    if rebuild {
        for path in planned["plan"]["unchanged"].as_array().ok_or(INDEX)? {
            let path = path.as_str().ok_or(INDEX)?;
            if index_extract::language(path).is_some() {
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
                    index_extract::language(p),
                    Some("typescript" | "tsx" | "javascript" | "jsx")
                )
            })
    {
        paths.insert(path.into());
    }
    let total = paths.len();
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
    let worker_count = parallel::workers();
    let selected: Vec<_> = paths.iter().take(max_files).collect();
    let mut prepared = std::collections::VecDeque::new();
    let mut preparation_micros = 0u128;
    let mut phase_totals = std::collections::BTreeMap::<String, u64>::new();
    let mut reference_attempts = 0u64;
    let mut reference_resolved = 0u64;
    let mut nodes_written = 0u64;
    #[cfg(windows)]
    let mut writer = None;
    for (position, path) in selected.iter().copied().enumerate() {
        if prepared.is_empty() && worker_count > 1 && context.check().is_ok() {
            let end = (position + worker_count).min(selected.len());
            let preparation_started = std::time::Instant::now();
            prepared.extend(parallel::prepare(root, &selected[position..end], context));
            preparation_micros += preparation_started.elapsed().as_micros();
        }
        let preparation = prepared.pop_front().flatten();
        context.report_progress("indexing", processed, Some(total), Some(path));
        let file_started = std::time::Instant::now();
        // Bound each file without extending the owner's deadline or losing cancellation.
        let file_context = context.with_execution_timeout(Some(std::time::Duration::from_secs(60)));
        sync_diagnostic::begin(path, &database);
        sync_diagnostic::stage("recheck_workspace_scope");
        let applied = scope.recheck(&project, &file_context).and_then(|()| {
            #[cfg(windows)]
            {
                sync_file_config_prepared(
                    root,
                    path,
                    None,
                    &file_context,
                    || {},
                    rebuild,
                    &mut writer,
                    preparation,
                )
            }
            #[cfg(not(windows))]
            {
                let _ = preparation;
                sync_file(root, path, &file_context)
            }
        });
        match applied {
            Ok(mut result) => {
                processed += 1;
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
                if let Some(diagnostic) = sync_diagnostic::take(code, message) {
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
    if failure.is_none()
        && let Err((code, message)) = scope.recheck(&project, context)
    {
        let entry = json!({"path":"codegraph.json","code":code,"message":message,"stage":"final_scope_recheck","commitState":"not_committed"});
        failures.push(entry.clone());
        failure = Some(entry);
    }
    let native_complete = failure.is_none() && processed == total;
    let mut report = json!({"mode":"sync_native_subset","rebuild":rebuild,"files":results,"failure":failure,"scope":scope.stamp(),
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
        report["phaseTimingsMicros"] = json!(phase_totals);
        report["preparationDurationMicros"] = json!(preparation_micros);
        report["totalReferencesAttempted"] = json!(reference_attempts);
        report["totalReferencesResolved"] = json!(reference_resolved);
        report["totalNodesWritten"] = json!(nodes_written);
    }
    Ok(report)
}

/// Consume contextual SQLite/index diagnostics for CLI and workspace reports.
pub fn take_sync_diagnostic(code: &str, message: &str) -> Option<Value> {
    sync_diagnostic::take(code, message)
}

pub(super) fn begin_plan_diagnostic(database: &Path) {
    sync_diagnostic::begin("<workspace-plan>", database);
    sync_diagnostic::stage("plan_workspace");
}

pub(super) fn sql<T>(value: rusqlite::Result<T>) -> Result<T, Failure> {
    value.map_err(|error| {
        sync_diagnostic::capture(&error);
        match error {
            rusqlite::Error::SqliteFailure(code, _) => index_failure_for_sqlite_code(code.code),
            _ => INDEX,
        }
    })
}

fn index_failure_for_sqlite_code(code: rusqlite::ErrorCode) -> Failure {
    match code {
        rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => INDEX_BUSY,
        _ => INDEX,
    }
}

#[cfg(test)]
#[path = "index_store/sql_error_mapping_tests.rs"]
mod sql_error_mapping_tests;
fn string(value: &Value, key: &str) -> Result<String, Failure> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 65536)
        .map(str::to_owned)
        .ok_or(INDEX)
}
fn opt_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, Failure> {
    value[key].as_array().ok_or(INDEX)
}

/// The writer deliberately supports only Windows, where ancestors and the
/// final DB entry can be pinned against replacement. The extraction core is portable.
pub fn sync_file(
    root: &Path,
    relative: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    #[cfg(not(windows))]
    {
        let _ = (root, relative, context);
        Err((
            "UNSUPPORTED",
            "Native index publication requires Windows path pinning",
        ))
    }
    #[cfg(windows)]
    {
        sync_file_windows(root, relative, context)
    }
}

/// Republishes one file even when its content did not change. The native
/// extractor runs again and the graph of that file is refreshed.
pub fn sync_file_forced(
    root: &Path,
    relative: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    #[cfg(not(windows))]
    {
        let _ = (root, relative, context);
        Err((
            "UNSUPPORTED",
            "Native index publication requires Windows path pinning",
        ))
    }
    #[cfg(windows)]
    {
        sync_file_config_force(root, relative, None, context, || {}, true)
    }
}

/// Explicit single tsconfig selection; no parent discovery or inherited configurations.
pub fn sync_file_with_tsconfig(
    root: &Path,
    relative: &str,
    config: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    #[cfg(windows)]
    {
        sync_file_config_with(root, relative, Some(config), context, || {})
    }
    #[cfg(not(windows))]
    {
        let _ = (root, relative, config, context);
        Err(("UNSUPPORTED", "Native index publication requires Windows"))
    }
}

#[cfg(windows)]
fn sync_file_windows(
    root: &Path,
    relative: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    sync_file_with(root, relative, context, || {})
}

#[cfg(windows)]
fn sync_file_with(
    root: &Path,
    relative: &str,
    context: &OperationContext,
    before_recheck: impl FnOnce(),
) -> Result<Value, Failure> {
    sync_file_config_with(root, relative, None, context, before_recheck)
}

#[cfg(windows)]
fn sync_file_config_with(
    root: &Path,
    relative: &str,
    requested_config: Option<&str>,
    context: &OperationContext,
    before_recheck: impl FnOnce(),
) -> Result<Value, Failure> {
    sync_file_config_force(
        root,
        relative,
        requested_config,
        context,
        before_recheck,
        false,
    )
}

/// Windows callers pass native separators; the graph stores POSIX-style paths.
fn normalize_relative(relative: &str) -> String {
    relative.replace('\\', "/")
}

#[cfg(windows)]
fn sync_file_config_force(
    root: &Path,
    relative: &str,
    requested_config: Option<&str>,
    context: &OperationContext,
    before_recheck: impl FnOnce(),
    force: bool,
) -> Result<Value, Failure> {
    context.report_progress("indexing", 0, Some(1), None);
    let result = sync_file_config_reuse(
        root,
        relative,
        requested_config,
        context,
        before_recheck,
        force,
        &mut None,
    );
    if result.is_ok() {
        context.report_progress("finalizing", 1, Some(1), None);
    }
    result
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)] // Explicit-file inputs plus operation-scoped connection ownership.
fn sync_file_config_reuse(
    root: &Path,
    relative: &str,
    requested_config: Option<&str>,
    context: &OperationContext,
    before_recheck: impl FnOnce(),
    force: bool,
    writer: &mut Option<writer::Writer>,
) -> Result<Value, Failure> {
    sync_file_config_prepared(
        root,
        relative,
        requested_config,
        context,
        before_recheck,
        force,
        writer,
        None,
    )
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn sync_file_config_prepared(
    root: &Path,
    relative: &str,
    requested_config: Option<&str>,
    context: &OperationContext,
    before_recheck: impl FnOnce(),
    force: bool,
    writer: &mut Option<writer::Writer>,
    prepared: Option<parallel::Prepared>,
) -> Result<Value, Failure> {
    context.check()?;
    index_extract::take_diagnostic();
    // Windows callers pass native separators; the graph stores POSIX-style paths.
    let normalized = normalize_relative(relative);
    let relative = normalized.as_str();
    if relative.is_empty()
        || relative.len() > 1000
        || relative.contains('\\')
        || relative.chars().any(char::is_control)
        || relative.split('/').any(|s| matches!(s, "" | "." | ".."))
        || index_inventory::restricted(relative)
    {
        return Err(("PATH", "Invalid or restricted synchronization source path"));
    }
    let language = index_extract::language(relative).ok_or((
        "SYNC_LANGUAGE",
        "No native extractor for this source language",
    ))?;
    let project = Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?;
    let database = project.root.join(".codegraph/codegraph.db");
    sync_diagnostic::begin(relative, &database);
    sync_diagnostic::stage("load_file_scope");
    let scope = scope::Snapshot::load(&project, context)?;
    sync_diagnostic::stage("read_source");
    let source = if scope.excluded(relative) {
        None
    } else {
        read_source(&project, relative, context)?
    };
    let reused = writer.is_some();
    if writer.is_none() {
        sync_diagnostic::stage("open_database");
        *writer = Some(writer::Writer::open(&project, context)?);
    }
    let writer = writer.as_mut().ok_or(INDEX)?;
    sync_diagnostic::stage("recheck_database_handle");
    writer.recheck(context)?;
    let db = &mut writer.db;
    let retry_memo = &mut writer.retry_memo;
    let cancel = context.clone();
    sync_diagnostic::stage("configure_connection");
    sql(db.progress_handler(1000, Some(move || cancel.check().is_err())))?;
    sql(db.execute_batch("PRAGMA trusted_schema=OFF;"))?;
    sync_diagnostic::stage("begin_transaction");
    let tx = sql(db.transaction_with_behavior(TransactionBehavior::Immediate))?;
    let run = || {
        let data_version: i64 = sql(tx.query_row("PRAGMA data_version", [], |row| row.get(0)))?;
        retry_memo.observe_version(data_version);
        sync_diagnostic::stage("validate_schema");
        validate_schema(&tx)?;
        // Only the three exact provider FTS triggers are accepted. SQLite FTS5
        // triggers require trusted_schema; it is enabled only after validation.
        sql(tx.execute_batch("PRAGMA trusted_schema=ON"))?;
        context.check()?;
        sync_diagnostic::stage("select_configuration");
        let configuration = tsconfig::Snapshot::select(&tx, &project, requested_config, context)?;
        let (mut result, import_sources) = publish(
            &tx,
            &project,
            relative,
            language,
            source.as_deref(),
            context,
            configuration,
            force,
            prepared,
            retry_memo,
        )?;
        result["tsconfig"] = import_sources.config.stamp();
        sync_diagnostic::stage("recheck_sources");
        before_recheck();
        scope.recheck(&project, context)?;
        import_sources.recheck(&project, context)?;
        let observed = if scope.excluded(relative) {
            None
        } else {
            read_source(&project, relative, context)?
        };
        if observed.as_ref().map(|b| hash(b)) != source.as_ref().map(|b| hash(b)) {
            return Err((
                "STALE",
                "Source changed before index publication; transaction rolled back",
            ));
        }
        context.check()?;
        // The publication only writes the edges of this file, so the integrity
        // check is scoped to them. A full database scan would make every
        // publication cost grow with the size of the project.
        sync_diagnostic::stage("integrity_check");
        let violations: bool = result["changed"] == true
            && sql(tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM edges e LEFT JOIN nodes s ON s.id=e.source LEFT JOIN nodes t ON t.id=e.target WHERE (e.source IN(SELECT id FROM nodes WHERE file_path=?1) OR e.target IN(SELECT id FROM nodes WHERE file_path=?1)) AND (s.id IS NULL OR t.id IS NULL))",
                [relative],
                |row| row.get(0),
            ))?;
        if violations {
            return Err(("SYNC_INTEGRITY", "Index foreign-key integrity check failed"));
        }
        result["sourceSnapshotAtomic"] = json!(false);
        result["writerConnectionReused"] = json!(reused);
        Ok(result)
    };
    let prepared = run();
    match prepared {
        Err(failure) => {
            retry_memo.clear();
            // Cancellation must not prevent the rollback itself.
            tx.progress_handler(0, None::<fn() -> bool>)
                .map_err(|_| UNCERTAIN)?;
            tx.rollback().map_err(|_| UNCERTAIN)?;
            Err(context.check().err().unwrap_or(failure))
        }
        Ok(mut result) => {
            // Commit point: don't report a post-commit cancellation as no write.
            tx.progress_handler(0, None::<fn() -> bool>)
                .map_err(|_| UNCERTAIN)?;
            sync_diagnostic::stage("commit_transaction");
            tx.commit().map_err(|error| {
                retry_memo.clear();
                sync_diagnostic::capture(&error);
                UNCERTAIN
            })?;
            result["committed"] = json!(true);
            if let Some(profile) = sync_diagnostic::finish_profile() {
                result["phaseTimingsMicros"] = profile;
            }
            Ok(result)
        }
    }
}

fn read_source(
    project: &Project,
    path: &str,
    context: &OperationContext,
) -> Result<Option<Vec<u8>>, Failure> {
    context.check()?;
    let target = project.target_optional(path)?;
    match std::fs::symlink_metadata(&target) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(("IO", "Source metadata unavailable; absence not established")),
        Ok(metadata) if !metadata.is_file() => {
            return Err(("PATH", "Source is not a regular file"));
        }
        Ok(_) => {}
    }
    #[cfg(windows)]
    let _pins = super::edit_commit::pin_directory(project, target.parent().ok_or(INDEX)?)?;
    project
        .bytes_with_limit(path, context, index_sync::MAX_SOURCE_BYTES as u64)
        .map(Some)
}
