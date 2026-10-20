//! Atomic, explicit-file CodeGraph publication. No migrations or source writes.
use super::{index_extract, index_inventory, index_sync, project::Project};
use harness_domain::{Failure, execution::OperationContext, hash};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const INDEX: Failure = (
    "SYNC_INDEX",
    "CodeGraph index incompatible, busy or invalid; no raw database diagnostics returned",
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
#[path = "index_razor_receivers.rs"]
mod razor_receivers;
#[path = "index_paths.rs"]
mod paths;
#[path = "index_rust.rs"]
mod rust;
#[path = "index_tsconfig.rs"]
pub(super) mod tsconfig;

#[path = "index_semantic_policy.rs"]
mod semantic_policy;

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
    context.check()?;
    let project = Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?;
    let scope = scope::Snapshot::load(&project, context)?;
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
    let mut results = Vec::new();
    let mut processed = 0;
    let mut changed = 0;
    let mut provider_dependent = 0;
    let mut failed = 0;
    let mut failure = None;
    #[cfg(windows)]
    let mut writer = None;
    for path in paths.iter().take(max_files) {
        // Bound each file without extending the owner's deadline or losing cancellation.
        let file_context = context.with_execution_timeout(Some(std::time::Duration::from_secs(60)));
        let applied = scope.recheck(&project, &file_context).and_then(|()| {
            #[cfg(windows)]
            {
                sync_file_config_reuse(root, path, None, &file_context, || {}, rebuild, &mut writer)
            }
            #[cfg(not(windows))]
            {
                sync_file(root, path, &file_context)
            }
        });
        match applied {
            Ok(result) => {
                processed += 1;
                changed += usize::from(result["changed"] == true);
                if results.len() < 32 {
                    results.push(result);
                }
            }
            Err((code, message)) => {
                let entry = json!({"path":path,"code":code,"message":message,"deadlineScope":"file_within_workspace","fileTimeoutSeconds":60,"commitState":if code==UNCERTAIN.0 {"unknown"}else{"not_committed"}});
                // Provider-dependent files (framework or synthesized graph data
                // the native publisher cannot reproduce) are a documented
                // limitation: they are reported and skipped.
                if code == "SYNC_UNSUPPORTED_RELATION" || code == "SYNC_LEGACY_REFERENCE" {
                    provider_dependent += 1;
                    unsupported += 1;
                    if results.len() < 32 {
                        results.push(json!({"path":path,"skipped":true,"code":code,"message":message,"commitState":"not_committed"}));
                    }
                    continue;
                }
                // A per-file failure does not stop the rest of the workspace:
                // the first failure is reported and the remaining files are
                // still published. Systemic failures still stop the run.
                if failure.is_none() {
                    failure = Some(entry.clone());
                }
                if results.len() < 32 {
                    results.push(entry);
                }
                if matches!(
                    code,
                    "SYNC_SCHEMA"
                        | "SYNC_INTEGRITY"
                        | "SYNC_COMMIT_UNCERTAIN"
                        | "SYNC_LIMIT"
                        | "LIMIT"
                        | "PATH"
                ) {
                    break;
                }
                failed += 1;
                continue;
            }
        }
    }
    if failure.is_none()
        && let Err((code, _)) = scope.recheck(&project, context)
    {
        failure = Some(json!({"path":"codegraph.json","code":code,"commitState":"not_committed"}));
    }
    let native_complete = failure.is_none() && processed == total;
    Ok(
        json!({"mode":"sync_native_subset","rebuild":rebuild,"files":results,"failure":failure,"scope":scope.stamp(),
        "changedFilesCommitted":changed,"nativeCandidates":total,"nativeFilesRemaining":total.saturating_sub(processed).saturating_sub(provider_dependent),
        "filesProcessed":processed,"fileReportsOmitted":processed.saturating_sub(results.len()),"fileReportLimit":32,
        "maxFiles":max_files,"selectedScopeApplied":native_complete,"workspaceTransactionAtomic":false,
        "unsupportedChangedFiles":unsupported,"uncheckedFiles":unchecked,"providerDependentFiles":provider_dependent,"skippedFiles":provider_dependent,"failedFiles":failed,
        "restrictedIndexedFiles":planned["inventory"]["restrictedIndexedFiles"],
        "restrictedIndexedReasons":planned["inventory"]["restrictedIndexedReasons"],
        "discoveryComplete":planned["inventory"]["discoveryComplete"],
        "synchronized":false,"globalFreshnessCertified":false,"fullProviderScope":false,
        "resolutionPolicy":"native_exact_name_and_relative_named_import_subset"}),
    )
}

fn sql<T>(value: rusqlite::Result<T>) -> Result<T, Failure> {
    value.map_err(|_| INDEX)
}
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
    sync_file_config_reuse(
        root,
        relative,
        requested_config,
        context,
        before_recheck,
        force,
        &mut None,
    )
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
    context.check()?;
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
    let scope = scope::Snapshot::load(&project, context)?;
    let source = if scope.excluded(relative) {
        None
    } else {
        read_source(&project, relative, context)?
    };
    let reused = writer.is_some();
    if writer.is_none() {
        *writer = Some(writer::Writer::open(&project, context)?);
    }
    let writer = writer.as_mut().ok_or(INDEX)?;
    writer.recheck(context)?;
    let db = &mut writer.db;
    let cancel = context.clone();
    sql(db.progress_handler(1000, Some(move || cancel.check().is_err())))?;
    sql(db.execute_batch("PRAGMA trusted_schema=OFF;"))?;
    let tx = sql(db.transaction_with_behavior(TransactionBehavior::Immediate))?;
    let run = || {
        validate_schema(&tx)?;
        // Only the three exact provider FTS triggers are accepted. SQLite FTS5
        // triggers require trusted_schema; it is enabled only after validation.
        sql(tx.execute_batch("PRAGMA trusted_schema=ON"))?;
        context.check()?;
        let configuration =
            tsconfig::Snapshot::select(&tx, &project, requested_config, context)?;
        let (mut result, import_sources) = publish(
            &tx,
            &project,
            relative,
            language,
            source.as_deref(),
            context,
            configuration,
            force,
        )?;
        result["tsconfig"] = import_sources.config.stamp();
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
            tx.commit().map_err(|_| UNCERTAIN)?;
            result["committed"] = json!(true);
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

fn normalized(sql: &str) -> String {
    sql.to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != ';')
        .collect::<String>()
        .replace("ifnotexists", "")
}
fn validate_schema(db: &Connection) -> Result<(), Failure> {
    let ordinary: i64 = sql(db.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('schema_versions','nodes','edges','files','unresolved_refs','project_metadata') AND type='table' AND upper(sql) LIKE 'CREATE TABLE%'", [], |r| r.get(0)))?;
    if ordinary != 6 {
        return Err(INDEX);
    }
    let version: i64 =
        sql(db.query_row("SELECT max(version) FROM schema_versions", [], |r| r.get(0)))?;
    if version != 11 {
        return Err(INDEX);
    }
    let mut statement =
        sql(db.prepare("SELECT name,sql FROM sqlite_schema WHERE type='trigger' LIMIT 4"))?;
    let rows = sql(statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }))?;
    let mut trigger_count = 0;
    for row in rows {
        let (name, body) = sql(row)?;
        let expected = match name.as_str() {
            "nodes_ai" => {
                "CREATE TRIGGER nodes_ai AFTER INSERT ON nodes BEGIN INSERT INTO nodes_fts(rowid,id,name,qualified_name,docstring,signature) VALUES(NEW.rowid,NEW.id,NEW.name,NEW.qualified_name,NEW.docstring,NEW.signature); END"
            }
            "nodes_ad" => {
                "CREATE TRIGGER nodes_ad AFTER DELETE ON nodes BEGIN INSERT INTO nodes_fts(nodes_fts,rowid,id,name,qualified_name,docstring,signature) VALUES('delete',OLD.rowid,OLD.id,OLD.name,OLD.qualified_name,OLD.docstring,OLD.signature); END"
            }
            "nodes_au" => {
                "CREATE TRIGGER nodes_au AFTER UPDATE ON nodes BEGIN INSERT INTO nodes_fts(nodes_fts,rowid,id,name,qualified_name,docstring,signature) VALUES('delete',OLD.rowid,OLD.id,OLD.name,OLD.qualified_name,OLD.docstring,OLD.signature); INSERT INTO nodes_fts(rowid,id,name,qualified_name,docstring,signature) VALUES(NEW.rowid,NEW.id,NEW.name,NEW.qualified_name,NEW.docstring,NEW.signature); END"
            }
            _ => {
                return Err((
                    "SYNC_SCHEMA",
                    "Unrecognized index trigger; publication refused",
                ));
            }
        };
        if normalized(&body) != normalized(expected) {
            return Err(("SYNC_SCHEMA", "Modified index trigger; publication refused"));
        }
        trigger_count += 1;
    }
    if trigger_count != 3 {
        return Err(("SYNC_SCHEMA", "Provider full-text triggers are missing"));
    }
    // Refuse virtual/view substitutions of writable tables via the ordinary-table
    // check above. If FTS exists, require the provider's exact external-content ABI.
    let fts: Option<String> = sql(db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='nodes_fts'",
            [],
            |r| r.get(0),
        )
        .optional())?;
    if let Some(fts) = fts {
        let expected = "CREATE VIRTUAL TABLE nodes_fts USING fts5(id,name,qualified_name,docstring,signature,content='nodes',content_rowid='rowid')";
        if normalized(&fts) != normalized(expected) {
            return Err(("SYNC_SCHEMA", "Unsupported full-text index definition"));
        }
    } else {
        return Err(("SYNC_SCHEMA", "Provider full-text index is missing"));
    }
    Ok(())
}

#[derive(Clone)]
struct Identity {
    id: String,
    key: String,
}
fn identities(db: &Connection, path: &str) -> Result<Vec<Identity>, Failure> {
    let mut statement = sql(db.prepare("SELECT id,kind,qualified_name,ifnull(signature,'') FROM nodes WHERE file_path=?1 LIMIT 20001"))?;
    let rows = sql(statement.query_map([path], |r| {
        Ok(Identity {
            id: r.get(0)?,
            key: format!(
                "{}\0{}\0{}",
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?
            ),
        })
    }))?;
    let mut result = Vec::new();
    let mut reserved = 0usize;
    for row in rows {
        let row = sql(row)?;
        reserved += row.id.len() + row.key.len();
        if result.len() >= MAX_ROWS || reserved > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        result.push(row);
    }
    Ok(result)
}
fn identity_map(
    prior: Vec<Identity>,
    extracted: Option<&Value>,
) -> Result<BTreeMap<String, String>, Failure> {
    let mut before: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut after: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for node in prior {
        before.entry(node.key).or_default().push(node.id);
    }
    if let Some(value) = extracted {
        for node in array(value, "nodes")? {
            let key = format!(
                "{}\0{}\0{}",
                string(node, "kind")?,
                string(node, "qualifiedName")?,
                opt_string(node, "signature").unwrap_or("")
            );
            after.entry(key).or_default().push(string(node, "id")?);
        }
    }
    let mut result = BTreeMap::new();
    for (key, ids) in before {
        if ids.len() == 1
            && let Some(next) = after.get(&key)
            && next.len() == 1
        {
            result.insert(ids[0].clone(), next[0].clone());
        }
    }
    Ok(result)
}

/// Pending JS/TS references whose imported export became available. The
/// reference names the local alias, so the import binding of the referencing
/// file carries the real name to match.
fn pending_import_refs(
    db: &Connection,
    new_names: &BTreeSet<String>,
) -> Result<Vec<i64>, Failure> {
    if new_names.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = sql(db.prepare(
        "SELECT r.id,r.reference_name,n.signature FROM unresolved_refs r JOIN nodes n ON n.file_path=r.file_path AND n.kind='import' WHERE r.status='pending' AND r.language IN ('typescript','tsx','javascript','jsx') LIMIT 20001",
    ))?;
    let rows = sql(statement.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    }))?;
    let mut result = Vec::new();
    for row in rows {
        let (id, reference, signature) = sql(row)?;
        let Some(signature) = signature else {
            continue;
        };
        let Some((_module, names)) = codegraph_native::named_imports(&signature) else {
            continue;
        };
        for (original, local) in names {
            if local == reference && new_names.contains(&original) {
                result.push(id);
                break;
            }
        }
    }
    Ok(result)
}
#[derive(Clone)]
struct ProviderEdge {
    source: String,
    target: String,
    kind: String,
    line: Option<i64>,
    col: Option<i64>,
    metadata: Option<String>,
    provenance: Option<String>,
    source_path: String,
    target_path: String,
}
/// Provider-owned framework nodes of one file. The native extractor never
/// produces them, so the publication keeps them instead of dropping them.
fn provider_node_ids(db: &Connection, path: &str) -> Result<BTreeSet<String>, Failure> {
    let mut statement = sql(db.prepare(
        "SELECT id FROM nodes WHERE file_path=?1 AND kind IN('route','component') LIMIT 20001",
    ))?;
    let rows = sql(statement.query_map([path], |r| r.get::<_, String>(0)))?;
    let mut result = BTreeSet::new();
    for row in rows {
        let row = sql(row)?;
        if result.len() >= MAX_ROWS {
            return Err(LIMIT);
        }
        result.insert(row);
    }
    Ok(result)
}
/// Edges incident to the provider nodes of one file, with the source file of
/// every edge so the publication can tell provider-owned edges from the
/// incoming edges it already retains.
fn provider_edges(db: &Connection, path: &str) -> Result<Vec<ProviderEdge>, Failure> {
    let mut statement = sql(db.prepare(
        "SELECT e.source,e.target,e.kind,e.line,e.col,e.metadata,e.provenance,ifnull(s.file_path,''),ifnull(t.file_path,'') FROM edges e LEFT JOIN nodes s ON s.id=e.source LEFT JOIN nodes t ON t.id=e.target WHERE e.source IN(SELECT id FROM nodes WHERE file_path=?1 AND kind IN('route','component')) OR e.target IN(SELECT id FROM nodes WHERE file_path=?1 AND kind IN('route','component')) LIMIT 20001",
    ))?;
    let rows = sql(statement.query_map([path], |r| {
        Ok(ProviderEdge {
            source: r.get(0)?,
            target: r.get(1)?,
            kind: r.get(2)?,
            line: r.get(3)?,
            col: r.get(4)?,
            metadata: r.get(5)?,
            provenance: r.get(6)?,
            source_path: r.get(7)?,
            target_path: r.get(8)?,
        })
    }))?;
    let mut result = Vec::new();
    for row in rows {
        let row = sql(row)?;
        if result.len() >= MAX_ROWS {
            return Err(LIMIT);
        }
        result.push(row);
    }
    Ok(result)
}
#[derive(Clone)]
struct Incoming {
    source: String,
    target: String,
    kind: String,
    line: Option<i64>,
    col: Option<i64>,
    metadata: Option<String>,
    provenance: Option<String>,
    path: String,
    language: String,
}
fn incoming(db: &Connection, path: &str) -> Result<Vec<Incoming>, Failure> {
    let mut statement = sql(db.prepare("SELECT e.source,e.target,e.kind,e.line,e.col,e.metadata,e.provenance,s.file_path,s.language FROM edges e JOIN nodes t ON t.id=e.target JOIN nodes s ON s.id=e.source WHERE t.file_path=?1 AND s.file_path!=?1 AND e.kind!='contains' LIMIT 20001"))?;
    let rows = sql(statement.query_map([path], |r| {
        Ok(Incoming {
            source: r.get(0)?,
            target: r.get(1)?,
            kind: r.get(2)?,
            line: r.get(3)?,
            col: r.get(4)?,
            metadata: r.get(5)?,
            provenance: r.get(6)?,
            path: r.get(7)?,
            language: r.get(8)?,
        })
    }))?;
    let mut result = Vec::new();
    let mut reserved = 0usize;
    for row in rows {
        let row = sql(row)?;
        reserved += row.source.len()
            + row.target.len()
            + row.kind.len()
            + row.path.len()
            + row.language.len()
            + row.metadata.as_ref().map_or(0, String::len)
            + row.provenance.as_ref().map_or(0, String::len);
        if result.len() >= MAX_ROWS || reserved > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        result.push(row);
    }
    Ok(result)
}
#[allow(clippy::too_many_arguments)] // Mirrors the provider's unresolved_refs row.
fn insert_ref(
    db: &Connection,
    source: &str,
    name: &str,
    kind: &str,
    line: i64,
    col: i64,
    candidates: Option<String>,
    origin: (&str, &str),
) -> Result<i64, Failure> {
    sql(sql(db.prepare_cached("INSERT INTO unresolved_refs(from_node_id,reference_name,reference_kind,line,col,candidates,file_path,language,status,name_tail) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending','')"))?.execute(params![source,name,kind,line,col,candidates,origin.0,origin.1]))?;
    Ok(db.last_insert_rowid())
}

#[allow(clippy::too_many_arguments)] // Publication inputs plus the transaction's configuration snapshot.
fn publish(
    db: &Transaction<'_>,
    project: &Project,
    path: &str,
    language: &str,
    bytes: Option<&[u8]>,
    context: &OperationContext,
    configuration: (tsconfig::Snapshot, bool),
    force: bool,
) -> Result<(Value, imports::Sources), Failure> {
    let (configuration, config_changed) = configuration;
    let record: Option<(String, Option<String>)> = sql(db
        .query_row(
            "SELECT content_hash,errors FROM files WHERE path=?1",
            [path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional())?;
    let clean = record.as_ref().is_some_and(|(_, errors)| {
        errors.as_ref().is_none_or(|e| {
            serde_json::from_str::<Value>(e).is_ok_and(|v| v.as_array().is_some_and(Vec::is_empty))
        })
    });
    if !config_changed
        && !force
        && ((bytes.is_none() && record.is_none())
            || (clean
                && bytes.is_some_and(|bytes| {
                    record
                        .as_ref()
                        .is_some_and(|(digest, _)| *digest == hash(bytes))
                })))
    {
        return Ok((
            json!({"mode":"sync_file","path":path,"changed":false,"extractionPerformed":false,"committed":false,"synchronized":false,"globalFreshnessCertified":false}),
            imports::Sources::configured(configuration),
        ));
    }
    // Parse only after the authoritative transaction sees a changed digest or
    // an extraction retry/configuration change. Duplicate watcher events are cheap.
    let extracted_value = bytes
        .map(|bytes| {
            let text =
                std::str::from_utf8(bytes).map_err(|_| ("ENCODING", "Source is not UTF-8"))?;
            index_extract::extract(path, text, context)
        })
        .transpose()?;
    let extracted = extracted_value.as_ref();
    let razor_changes = if language == "csharp" {
        Some(razor_invalidation::Changes::capture(db, path, extracted)?)
    } else {
        None
    };
    // Provider-owned framework nodes (routes, components) and the edges the
    // provider synthesized around them cannot be reproduced by the native
    // extractor. The publication keeps them and writes the native nodes of
    // this file next to them. Only provider data that the native publisher
    // cannot reproduce at all still blocks the transaction.
    let provider_ids: BTreeSet<String> = provider_node_ids(db, path)?;
    let synthesized:bool = sql(db.query_row("SELECT EXISTS(SELECT 1 FROM edges WHERE (source IN(SELECT id FROM nodes WHERE file_path=?1 AND kind NOT IN('route','component')) OR target IN(SELECT id FROM nodes WHERE file_path=?1 AND kind NOT IN('route','component'))) AND CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.synthesizedBy') IS NOT NULL ELSE 0 END)",[path],|r|r.get(0)))?;
    let third_file:bool = sql(db.query_row("SELECT EXISTS(SELECT 1 FROM edges WHERE CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.synthesizedBy') END IS NOT NULL AND CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.registeredAt') END >= ?1 AND CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.registeredAt') END < ?2)",params![format!("{path}:"),format!("{path};")],|r|r.get(0)))?;
    let synthesis_table:bool = sql(db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='synthesis_inputs' AND upper(sql) LIKE 'CREATE TABLE%')",[],|r|r.get(0)))?;
    let _prior_synthesis_input = if synthesis_table {
        sql(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM synthesis_inputs WHERE file_path=?1)",
            [path],
            |r| r.get::<_, bool>(0),
        ))?
    } else {
        false
    };
    // A file the provider listed as a synthesis input can still be published
    // natively: its provider nodes and edges are preserved and the recorded
    // input stays, so the provider can synthesize again. Only a provider
    // relation that points at the native nodes of this file blocks it.
    if synthesized || third_file {
        return Err((
            "SYNC_UNSUPPORTED_RELATION",
            "Affected framework/synthesized graph data requires the provider resolver; index unchanged",
        ));
    }
    // Provider nodes and their edges survive the publication.
    let provider_edges: Vec<ProviderEdge> = provider_edges(db, path)?;
    let prior = identities(db, path)?;
    let paired = identity_map(prior, extracted)?;
    // A name that other files resolve against is a name this file publishes.
    // A pending ref can only be unblocked by a name that appears now or that
    // becomes published now.
    let mut prior_published: BTreeSet<String> = BTreeSet::new();
    {
        let mut statement = sql(db.prepare(
            "SELECT name FROM nodes WHERE file_path=?1 AND ifnull(is_exported,0)=1 LIMIT 20001",
        ))?;
        let rows = sql(statement.query_map([path], |r| r.get::<_, String>(0)))?;
        for row in rows {
            prior_published.insert(sql(row)?);
        }
    }
    let incoming = incoming(db, path)?;
    // Capture original refs BEFORE deleting nodes/edges; never turn receiver.foo
    // into foo. Legacy unpaired edges without stamps block the entire transaction.
    let mut resurrect = Vec::new();
    let mut retained = Vec::new();
    for mut edge in incoming {
        context.check()?;
        if provider_ids.contains(&edge.target) {
            // The provider node survives the publication; the edge keeps it.
            retained.push(edge);
            continue;
        }
        if let Some(target) = paired.get(&edge.target) {
            edge.target = target.clone();
            retained.push(edge);
        } else {
            let metadata: Value = edge
                .metadata
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok())
                .ok_or((
                    "SYNC_LEGACY_REFERENCE",
                    "Cannot recover original reference for an incoming edge; index unchanged",
                ))?;
            let name = string(&metadata, "refName").map_err(|_| {
                (
                    "SYNC_LEGACY_REFERENCE",
                    "Incoming edge lacks original reference metadata; index unchanged",
                )
            })?;
            let kind = opt_string(&metadata, "refKind")
                .unwrap_or(&edge.kind)
                .to_owned();
            resurrect.push((edge, name, kind));
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| INDEX)?
        .as_millis() as i64;
    let generated: i64 = sql(db
        .query_row("SELECT generated FROM files WHERE path=?1", [path], |r| {
            r.get(0)
        })
        .optional())?
    .unwrap_or(0);
    // Explicit deletion also supports indexes produced with foreign_keys off.
    // Provider nodes and their pending references survive the publication.
    sql(db.execute("DELETE FROM unresolved_refs WHERE from_node_id IN(SELECT id FROM nodes WHERE file_path=?1 AND kind NOT IN('route','component'))",[path]))?;
    sql(db.execute("DELETE FROM edges WHERE source IN(SELECT id FROM nodes WHERE file_path=?1) OR target IN(SELECT id FROM nodes WHERE file_path=?1)",[path]))?;
    sql(db.execute("DELETE FROM nodes WHERE file_path=?1 AND kind NOT IN('route','component')", [path]))?;
    let mut refs = BTreeSet::new();
    let mut node_count = 0;
    if let Some(extracted) = extracted {
        let nodes = array(extracted, "nodes")?;
        if nodes.len() > MAX_ROWS {
            return Err(LIMIT);
        }
        batches::insert(
            db,
            "INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,docstring,signature,visibility,is_exported,is_async,is_static,is_abstract,decorators,type_parameters,return_type,updated_at) VALUES ",
            21,
            nodes,
            context,
            |node| {
                batches::values(params![
                    string(node, "id")?,
                    string(node, "kind")?,
                    string(node, "name")?,
                    string(node, "qualifiedName")?,
                    path,
                    language,
                    node["startLine"].as_i64(),
                    node["endLine"].as_i64(),
                    node["startColumn"].as_i64(),
                    node["endColumn"].as_i64(),
                    opt_string(node, "docstring"),
                    opt_string(node, "signature"),
                    opt_string(node, "visibility"),
                    node["isExported"].as_bool().unwrap_or(false),
                    node["isAsync"].as_bool().unwrap_or(false),
                    node["isStatic"].as_bool().unwrap_or(false),
                    node["isAbstract"].as_bool().unwrap_or(false),
                    node.get("decorators").map(Value::to_string),
                    node.get("typeParameters").map(Value::to_string),
                    opt_string(node, "returnType"),
                    now
                ])
            },
        )?;
        node_count = nodes.len() + provider_ids.len();
        batches::insert(
            db,
            "INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES ",
            7,
            array(extracted, "edges")?,
            context,
            |edge| {
                batches::values(params![
                    string(edge, "source")?,
                    string(edge, "target")?,
                    string(edge, "kind")?,
                    edge["line"].as_i64(),
                    edge["column"].as_i64(),
                    edge.get("metadata").map(Value::to_string),
                    opt_string(edge, "provenance")
                ])
            },
        )?;
        refs.extend(batches::insert_ids(
            db,
            "INSERT INTO unresolved_refs(from_node_id,reference_name,reference_kind,line,col,candidates,file_path,language) VALUES ",
            8,
            array(extracted, "unresolvedReferences")?,
            context,
            |reference| batches::values(params![
                string(reference, "fromNodeId")?,
                string(reference, "referenceName")?,
                string(reference, "referenceKind")?,
                reference["line"].as_i64().ok_or(INDEX)?,
                reference["column"].as_i64().ok_or(INDEX)?,
                reference.get("candidates").map(Value::to_string),
                path, language,
            ]),
        )?);
        if refs.len() > MAX_ROWS {
            return Err(LIMIT);
        }
        let bytes = bytes.ok_or(INDEX)?;
        // modified_at=0 disables CodeGraph's size/mtime fast-path on the next
        // provider sync; don't invent a file timestamp from extraction time.
        sql(db.execute("INSERT INTO files(path,content_hash,language,size,modified_at,indexed_at,node_count,errors,generated) VALUES(?1,?2,?3,?4,0,?5,?6,'[]',?7) ON CONFLICT(path) DO UPDATE SET content_hash=excluded.content_hash,language=excluded.language,size=excluded.size,modified_at=0,indexed_at=excluded.indexed_at,node_count=excluded.node_count,errors=excluded.errors,generated=excluded.generated",params![path,hash(bytes),language,bytes.len() as i64,now,node_count as i64,generated]))?;
    } else {
        sql(db.execute("DELETE FROM files WHERE path=?1", [path]))?;
    }
    let retained_count = retained.len();
    let resurrected_count = resurrect.len();
    batches::insert(
        db,
        "INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES ",
        7,
        &retained,
        context,
        |edge| {
            batches::values(params![
                edge.source,
                edge.target,
                edge.kind,
                edge.line,
                edge.col,
                edge.metadata,
                edge.provenance
            ])
        },
    )?;
    for (edge, name, kind) in resurrect {
        context.check()?;
        refs.insert(insert_ref(
            db,
            &edge.source,
            &name,
            &kind,
            edge.line.unwrap_or(0),
            edge.col.unwrap_or(0),
            None,
            (&edge.path, &edge.language),
        )?);
        if refs.len() > MAX_ROWS {
            return Err(LIMIT);
        }
    }
    // Provider-owned edges are republished with their native endpoints remapped.
    // Edges that arrive from another file are already retained above.
    let mut provider_kept = 0usize;
    let mut provider_dropped = 0usize;
    for edge in &provider_edges {
        context.check()?;
        if edge.source_path != path {
            continue;
        }
        let source = if edge.source_path != path {
            edge.source.clone()
        } else if provider_ids.contains(&edge.source) {
            edge.source.clone()
        } else if let Some(next) = paired.get(&edge.source) {
            next.clone()
        } else {
            provider_dropped += 1;
            continue;
        };
        let target = if edge.target_path != path {
            edge.target.clone()
        } else if provider_ids.contains(&edge.target) {
            edge.target.clone()
        } else if let Some(next) = paired.get(&edge.target) {
            next.clone()
        } else {
            provider_dropped += 1;
            continue;
        };
        let duplicate: bool = sql(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM edges WHERE source=?1 AND target=?2 AND kind=?3)",
            params![source, target, edge.kind],
            |r| r.get(0),
        ))?;
        if duplicate {
            continue;
        }
        sql(db.execute("INSERT INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![source,target,edge.kind,edge.line,edge.col,edge.metadata,edge.provenance]))?;
        provider_kept += 1;
    }
    // A publication only invalidates the whole import graph when the
    // configuration changed or the file content changed, that is when its
    // exports or its import specifiers may have changed. Otherwise only the
    // import edges this file can affect are rechecked, so the work stays
    // proportional to the file instead of the whole project.

    let import_rechecks = if config_changed || matches!(language, "typescript" | "tsx" | "javascript" | "jsx") {
        imports::invalidate(db)?
    } else {
        Vec::new()
    };
    let import_recheck_count = import_rechecks.len();
    refs.extend(import_rechecks);
    if language == "rust" {
        refs.extend(imports::invalidate_rust(db)?);
    }
    let razor_recheck_count = if let Some(changes) = &razor_changes {
        let rechecks = changes.recheck(db, context)?;
        let count = rechecks.len();
        refs.extend(rechecks);
        count
    } else {
        0
    };
    if refs.len() > MAX_ROWS {
        return Err(LIMIT);
    }
    // Retry the pending refs whose exact name became available in this file.
    // The failed ones are retried by the engine that produced them (the razor
    // tail index, the import invalidation). The project-wide import clause
    // only applies to a JS/TS publication: a C# or Razor source cannot change
    // the resolution of a JavaScript import.
    let mut statement=sql(db.prepare("SELECT id FROM unresolved_refs WHERE status='pending' AND (reference_name IN(SELECT name FROM nodes WHERE file_path=?1) OR reference_name IN(SELECT qualified_name FROM nodes WHERE file_path=?1) OR (?2 IN ('typescript','tsx','javascript','jsx') AND language IN ('typescript','tsx','javascript','jsx') AND file_path IN(SELECT file_path FROM nodes WHERE kind='import')) OR (?2='rust' AND language='rust' AND reference_name LIKE 'super::%' AND reference_kind IN ('calls','function_ref'))) LIMIT 20001"))?;
    for row in sql(statement.query_map(params![path, language], |r| r.get::<_, i64>(0)))? {
        refs.insert(sql(row)?);
        if refs.len() > MAX_ROWS {
            return Err(LIMIT);
        }
    }
    let mut candidate_snapshot = candidates::Snapshot::default();
    let mut resolved = 0;
    let mut import_sources = imports::Sources::configured(configuration);
    let mut mvc_snapshot = super::razor_mvc::Snapshot::default();
    for id in &refs {
        context.check()?;
        resolved += usize::from(resolve_exact(
            db,
            *id,
            project,
            context,
            &mut import_sources,
            &mut candidate_snapshot,
            &mut mvc_snapshot,
        )?);
    }
    if import_sources.config.digest.is_some()
        || import_sources
            .config
            .semantic
            .as_ref()
            .is_some_and(|p| p.stamp().is_some())
    {
        sql(db.execute("INSERT INTO project_metadata(key,value,updated_at) VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at", params![tsconfig::KEY, import_sources.config.stamp().to_string(), now]))?;
    } else {
        sql(db.execute("DELETE FROM project_metadata WHERE key=?1", [tsconfig::KEY]))?;
    }
    // Invalidate the global provider stamp. This single-file subset is not a
    // full language/framework resolver or a whole-workspace synchronization.
    sql(db.execute(
        "DELETE FROM project_metadata WHERE key IN('indexed_at_commit','indexed_dirty_paths')",
        [],
    ))?;
    sql(db.execute("INSERT INTO project_metadata(key,value,updated_at) VALUES('native_sync_state',?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",params![json!({"scope":"explicit_file","resolution":"exact_name_and_relative_named_import_subset","globalFreshnessCertified":false}).to_string(),now]))?;
    Ok((
        json!({"mode":"sync_file","path":path,"changed":true,"extractionPerformed":extracted.is_some(),"deleted":extracted.is_none(),"nodesWritten":node_count,"incomingEdgesRetained":retained_count,"incomingReferencesRecovered":resurrected_count,"providerNodesPreserved":provider_ids.len(),"providerEdgesRetained":provider_kept,"providerEdgesDropped":provider_dropped,"importReferencesRechecked":import_recheck_count,"razorReferencesRechecked":razor_recheck_count,"referencesAttempted":refs.len(),"referencesResolved":resolved,"referencesPending":refs.len()-resolved,"resolutionPolicy":"native_exact_name_and_relative_named_import_subset","committed":false,"synchronized":false,"globalFreshnessCertified":false}),
        import_sources,
    ))
}

type ResolutionReference = (String, String, String, i64, i64, String, String, String);
fn resolve_exact(
    db: &Connection,
    id: i64,
    project: &Project,
    context: &OperationContext,
    sources: &mut imports::Sources,
    candidate_snapshot: &mut candidates::Snapshot,
    mvc_snapshot: &mut super::razor_mvc::Snapshot,
) -> Result<bool, Failure> {
    let reference:Option<ResolutionReference> = sql(sql(db.prepare_cached("SELECT r.from_node_id,r.reference_name,r.reference_kind,r.line,r.col,r.file_path,r.language,n.qualified_name FROM unresolved_refs r JOIN nodes n ON n.id=r.from_node_id WHERE r.id=?1"))?.query_row([id],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional())?;
    let Some((from, name, kind, line, col, path, language, owner)) = reference else {
        return Ok(false);
    };
    if language == "razor" && kind == "mvc_route" {
        let Some(target) = mvc_snapshot.resolve(db, &name, &path, context)? else {
            razor_invalidation::mark_failed(db, id, &name, &kind)?;
            return Ok(false);
        };
        let metadata = json!({"refName":name,"refKind":kind,
            "resolvedBy":"razor-mvc-literal-route-subset-v1",
            "confidence":0.9}).to_string();
        sql(db.execute("INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,'calls',?3,?4,?5,'heuristic')", params![from,target,line,col,metadata]))?;
        sql(db.execute("DELETE FROM unresolved_refs WHERE id=?1", [id]))?;
        return Ok(true);
    }
    if language == "razor" && kind == "calls" {
        // The tail index permits exact affected-name retries after C# edits.
        razor_invalidation::mark_failed(db, id, &name, &kind)?;
    }
    let allowed: &[&str] = match kind.as_str() {
        "calls" | "function_ref" => &["function", "method"],
        "instantiates" => &["class", "struct"],
        "extends" | "implements" => &["class", "struct", "interface", "trait"],
        "references" if language == "razor" => &["class", "struct", "record", "interface", "enum"],
        _ => return Ok(false),
    };
    let js = matches!(
        language.as_str(),
        "typescript" | "tsx" | "javascript" | "jsx"
    );
    if js
        && sources
            .config
            .semantic
            .as_ref()
            .is_some_and(|policy| policy.excluded(&path))
    {
        return Ok(false);
    }
    let mut scope = if js {
        imports::scope(db, &path, &name, &language, context, sources)?
    } else {
        imports::Scope::None
    };
    if matches!(scope, imports::Scope::Pending) {
        return Ok(false);
    }
    let mut export_contributors = Vec::new();
    if let imports::Scope::Binding {
        path: target,
        name: exported,
    } = &scope
    {
        if kind != "calls" || !sources.permits(db, project, context, &path, &name, line, col)? {
            return Ok(false);
        }
        let Some(target) = sources.export_target(db, project, context, target, exported)? else {
            return Ok(false);
        };
        export_contributors = target.contributors;
        scope = imports::Scope::Binding {
            path: target.path,
            name: target.name,
        };
    }
    let (lookup, imported_path) = match &scope {
        imports::Scope::Binding { path, name } => (name.as_str(), Some(path.as_str())),
        _ => (name.as_str(), None),
    };
    // The C# kernel qualifies the namespace/type boundary with `::`; Razor's
    // @model directive uses C# source spelling (`Demo.Model`).
    let model_lookup = if language == "razor" && kind == "references" {
        lookup
            .rsplit_once('.')
            .map(|(namespace, name)| format!("{namespace}::{name}"))
    } else {
        None
    };
    let lookup = model_lookup.as_deref().unwrap_or(lookup);
    // Receivers/chains are never stripped. Named imports override global names;
    // unsupported import forms defer instead of falling back to a homonym.
    let rust_target = if language == "rust" && matches!(kind.as_str(), "calls" | "function_ref") {
        let Some(target) = sources.rust_target(db, project, context, &path, &name, line, col)?
        else {
            return Ok(false);
        };
        Some(target)
    } else {
        None
    };
    // An absolute C# type receiver cannot be a local variable or a Razor
    // member. Resolve only unique public static methods; instance receivers,
    // aliases, unqualified names and overloads still need semantic binding.
    let razor_static_target = if language == "razor" && kind == "calls" {
        if let Some(absolute) = name.strip_prefix("global::") {
            if absolute.split('.').count() >= 2
                && absolute.split('.').all(|part| {
                    !part.is_empty()
                        && part.bytes().enumerate().all(|(i, b)| {
                            b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                        })
                })
            {
                let mut statement = sql(db.prepare_cached(
                    "SELECT id FROM nodes WHERE name=?2 COLLATE BINARY AND language='csharp' AND kind='method' AND visibility='public' AND is_static=1 AND replace(qualified_name,'::','.')=?1 COLLATE BINARY LIMIT 2",
                ))?;
                let matches = sql(statement.query_map(
                    [absolute, absolute.rsplit('.').next().unwrap_or("")],
                    |row| row.get::<_, String>(0),
                ))?
                .collect::<Result<Vec<_>, _>>();
                let matches = sql(matches)?;
                (matches.len() == 1).then(|| matches[0].clone())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    let razor_instance_target = if language == "razor" && kind == "calls" && razor_static_target.is_none() && name.contains('.') {
        sources.razor_instance_target(db, project, context, &path, &name, line, col)?
    } else { None };
    let mut candidates = if let Some((target, _)) = &razor_instance_target {
        vec![sql(db.query_row("SELECT id,kind,file_path,qualified_name,visibility,is_exported,language FROM nodes WHERE id=?1", [target], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))))?]
    } else if let Some(target) = &rust_target {
        vec![sql(db.query_row("SELECT id,kind,file_path,qualified_name,visibility,is_exported,language FROM nodes WHERE id=?1", [target], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))))?]
    } else if let Some(target) = &razor_static_target {
        vec![sql(db.query_row("SELECT id,kind,file_path,qualified_name,visibility,is_exported,language FROM nodes WHERE id=?1", [target], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))))?]
    } else {
        candidate_snapshot.lookup(
            db,
            lookup,
            if language == "razor" && kind == "references" {
                "csharp"
            } else {
                &language
            },
            imported_path,
            context,
        )?
    };
    if candidates.len() > 64 {
        return Ok(false);
    }
    candidates.retain(|n| allowed.contains(&n.1.as_str()));
    // Member blocks belong to the generated class of this view. A homonymous
    // method in another view is never proof of a Razor call target.
    if language == "razor"
        && matches!(kind.as_str(), "calls" | "function_ref")
        && razor_static_target.is_none()
        && razor_instance_target.is_none()
    {
        candidates.retain(|n| n.2 == path);
    }
    if let Some(target) = imported_path {
        candidates.retain(|n| n.2 == target && n.1 == "function" && n.3 == lookup);
    } else if matches!(scope, imports::Scope::LocalOnly) {
        candidates.retain(|n| n.2 == path);
    }
    // Cross-file private symbols require receiver/import/visibility analysis
    // outside this subset. Keep them pending rather than guessing accessibility.
    if imported_path.is_none() && rust_target.is_none() {
        candidates.retain(|n| n.2 == path || n.4.as_deref() == Some("public") || n.5);
    }
    let qualified = lookup.contains('.') || lookup.contains("::");
    if rust_target.is_some() || razor_static_target.is_some() || razor_instance_target.is_some() {
        // The language-specific proof already selected the exact declaration.
    } else if qualified {
        candidates.retain(|n| n.3 == lookup);
    } else if imported_path.is_none() {
        let container = owner
            .rsplit_once("::")
            .map(|(p, _)| format!("{p}::"))
            .or_else(|| owner.rsplit_once('.').map(|(p, _)| format!("{p}.")));
        if let Some(container) = container {
            let scoped = candidates
                .iter()
                .filter(|n| n.2 == path && n.3 == format!("{container}{name}"))
                .cloned()
                .collect::<Vec<_>>();
            if !scoped.is_empty() {
                candidates = scoped;
            }
        }
        let local = candidates
            .iter()
            .filter(|n| n.2 == path)
            .cloned()
            .collect::<Vec<_>>();
        if !local.is_empty() {
            candidates = local;
        }
    }
    if candidates.len() != 1 {
        return Ok(false);
    }
    let edge_kind = if kind == "function_ref" {
        "references"
    } else {
        &kind
    };
    let mut metadata = json!({"refName":name,"refKind":kind,"resolvedBy":if imported_path.is_some(){"native-relative-named-import-subset"}else{"native-exact-name-subset"},"confidence":0.9});
    if razor_static_target.is_some() {
        metadata["resolvedBy"] = json!("razor-absolute-static-method-subset-v1");
    }
    if let Some((_, receiver_type)) = &razor_instance_target {
        metadata["resolvedBy"] = json!("razor-typed-instance-method-subset-v1");
        metadata["receiverType"] = json!(receiver_type.strip_prefix("global::").unwrap_or(receiver_type));
    }
    if rust_target.is_some() {
        metadata["resolvedBy"] = json!(if candidates[0].2 == path {
            "syn-lexical-subset-v1"
        } else {
            "syn-module-subset-v1"
        });
    }
    if imported_path.is_some() {
        if sources.config.path.is_some() {
            metadata["tsconfig"] = sources.config.stamp();
        }
        metadata["bindingEngine"] = json!("oxc-semantic-0.90.0");
        metadata["moduleEngine"] = json!("oxc-resolver-11.0.0-index-snapshot");
        metadata["exportEngine"] = json!("oxc-module-record-0.90.0-resolve-export-v2");
        metadata["exportContributors"] = json!(export_contributors);
    }
    let metadata = metadata.to_string();
    sql(sql(db.prepare_cached("INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,?3,?4,?5,?6,'heuristic')"))?.execute(params![from,candidates[0].0,edge_kind,line,col,metadata]))?;
    sql(sql(db.prepare_cached("DELETE FROM unresolved_refs WHERE id=?1"))?.execute([id]))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn semantic_exclusion_keeps_symbols_and_prevents_deep_binding_failure() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let context = OperationContext::default();
        initialize(root, &context).unwrap();
        std::fs::create_dir(root.join(".agents")).unwrap();
        std::fs::create_dir_all(root.join("Scripts/vendor")).unwrap();
        std::fs::write(root.join("lib.js"), "export function run() {}\n").unwrap();
        sync_file(root, "lib.js", &context).unwrap();
        // Duplicate var declarations are valid legacy JS, but outside this resolver's
        // conservative semantic subset. Basic provider extraction still succeeds.
        let source = "import { run } from '../../lib.js'; var x = 1; var x = 2; export function plugin() { run(); }\n";
        std::fs::write(root.join("Scripts/vendor/plugin.js"), source).unwrap();
        let before = sync_file(root, "Scripts/vendor/plugin.js", &context).unwrap_err();
        assert_eq!(before.0, "SYNC_SEMANTIC");
        std::fs::write(
            root.join(".agents/harness.config.json"),
            r#"{"indexing":{"semanticExclude":["Scripts/vendor/**"]}}"#,
        )
        .unwrap();
        let report = sync_file(root, "Scripts/vendor/plugin.js", &context).unwrap();
        assert_eq!(report["changed"], true);
        // A second pass must also succeed rather than enter periodic retries.
        sync_file(root, "Scripts/vendor/plugin.js", &context).unwrap();
        let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
        let symbols: i64 = db.query_row("SELECT count(*) FROM nodes WHERE file_path='Scripts/vendor/plugin.js' AND name='plugin'", [], |r| r.get(0)).unwrap();
        assert_eq!(symbols, 1);
        let imported: i64 = db.query_row("SELECT count(*) FROM edges WHERE source IN (SELECT id FROM nodes WHERE file_path='Scripts/vendor/plugin.js') AND target IN (SELECT id FROM nodes WHERE file_path='lib.js')", [], |r| r.get(0)).unwrap();
        assert_eq!(imported, 0);
    }
    #[cfg(windows)]
    #[test]
    fn reused_writer_preserves_full_durability_and_rejects_late_links() {
        let root = tempfile::tempdir().unwrap();
        let context = OperationContext::default();
        super::initialize(root.path(), &context).unwrap();
        std::fs::write(root.path().join("a.rs"), "pub fn first() {}\n").unwrap();
        std::fs::write(root.path().join("b.rs"), "pub fn second() { first(); }\n").unwrap();
        let mut writer = None;
        let first = super::sync_file_config_reuse(
            root.path(),
            "a.rs",
            None,
            &context,
            || {},
            false,
            &mut writer,
        )
        .unwrap();
        let second = super::sync_file_config_reuse(
            root.path(),
            "b.rs",
            None,
            &context,
            || {},
            false,
            &mut writer,
        )
        .unwrap();
        assert_eq!(first["writerConnectionReused"], false);
        assert_eq!(second["writerConnectionReused"], true);
        let writer = writer.unwrap();
        assert_eq!(
            writer
                .db
                .query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            writer
                .db
                .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(writer.db.is_autocommit());
        // Some Windows filesystems/permissions do not allow this link operation.
        if std::fs::hard_link(
            root.path().join(".codegraph/codegraph.db"),
            root.path().join("alias.db"),
        )
        .is_ok()
        {
            assert_eq!(writer.recheck(&context).unwrap_err().0, "PATH");
        }
    }

    #[cfg(windows)]
    #[test]
    fn tsconfig_change_before_commit_rolls_back_nodes_and_configuration_stamp() {
        use super::*;
        for changed in ["tsconfig.json", "base.json"] {
            let dir = tempfile::tempdir().unwrap();
            fixture(dir.path());
            std::fs::write(dir.path().join("a.ts"), "export function planned() {}\n").unwrap();
            std::fs::write(
                dir.path().join("tsconfig.json"),
                "{\"compilerOptions\":{\"baseUrl\":\".\"}}",
            )
            .unwrap();
            if changed == "base.json" {
                std::fs::write(
                    dir.path().join("tsconfig.json"),
                    "{\"extends\":\"./base.json\"}",
                )
                .unwrap();
                std::fs::write(dir.path().join("base.json"), "{}").unwrap();
            }
            let result = sync_file_config_with(
                dir.path(),
                "a.ts",
                Some("tsconfig.json"),
                &OperationContext::default(),
                || {
                    std::fs::write(
                        dir.path().join(changed),
                        "{\"compilerOptions\":{\"baseUrl\":\"src\"}}",
                    )
                    .unwrap();
                },
            );
            assert_eq!(result.unwrap_err().0, "STALE");
            let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
            assert_eq!(
                db.query_row("SELECT count(*) FROM nodes WHERE name='planned'", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM project_metadata WHERE key='native_tsconfig_v1'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        }
    }
    use super::*;
    fn fixture(root: &Path) {
        std::fs::create_dir_all(root.join(".codegraph")).unwrap();
        let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
        db.execute_batch(include_str!("../../vendor/codegraph-native/schema.sql"))
            .unwrap();
        db.execute_batch("INSERT INTO schema_versions(version,applied_at) VALUES(11,0); PRAGMA journal_mode=WAL;").unwrap();
        std::fs::write(root.join("a.rs"), "pub fn old() {}\n").unwrap();
        sync_file(root, "a.rs", &OperationContext::default()).unwrap();
    }
    #[test]
    fn source_change_after_sql_writes_rolls_back_index() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        std::fs::write(dir.path().join("a.rs"), "pub fn planned() {}\n").unwrap();
        let result = sync_file_with(dir.path(), "a.rs", &OperationContext::default(), || {
            std::fs::write(dir.path().join("a.rs"), "pub fn concurrent() {}\n").unwrap();
        });
        assert_eq!(result.unwrap_err().0, "STALE");
        let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM nodes WHERE name='old'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM nodes WHERE name='planned'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn cancellation_after_sql_writes_does_not_interrupt_rollback() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        std::fs::write(dir.path().join("a.rs"), "pub fn cancelled() {}\n").unwrap();
        let context = OperationContext::default();
        let result = sync_file_with(dir.path(), "a.rs", &context, || context.cancel());
        assert_eq!(result.unwrap_err().0, "CANCELLED");
        let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM nodes WHERE name='old'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn star_nonwinning_branch_changed_before_commit_rolls_back_target_update() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        for (path, source) in [
            ("lib.ts", "export function api(){}"),
            ("other.ts", "export function unrelated(){}"),
            (
                "barrel.ts",
                "export * from './lib'; export * from './other';",
            ),
            (
                "app.ts",
                "import {api} from './barrel'; export function caller(){api();}",
            ),
        ] {
            std::fs::write(dir.path().join(path), source).unwrap();
            sync_file(dir.path(), path, &OperationContext::default()).unwrap();
        }
        let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
        let before: String = db
            .query_row(
                "SELECT content_hash FROM files WHERE path='lib.ts'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        drop(db);
        std::fs::write(dir.path().join("lib.ts"), "\nexport function api(){}").unwrap();
        let result = sync_file_with(dir.path(), "lib.ts", &OperationContext::default(), || {
            std::fs::write(dir.path().join("other.ts"), "export function api(){}").unwrap();
        });
        assert_eq!(result.unwrap_err().0, "STALE");
        let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT content_hash FROM files WHERE path='lib.ts'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            before
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM edges WHERE kind='calls'", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            1
        );
    }
    #[test]
    fn barrel_changed_after_resolution_rolls_back_target_update() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        for (path, source) in [
            ("lib.ts", "export function run(){}"),
            ("barrel.ts", "export {run as api} from './lib';"),
            (
                "app.ts",
                "import {api} from './barrel'; export function caller(){api();}",
            ),
        ] {
            std::fs::write(dir.path().join(path), source).unwrap();
            sync_file(dir.path(), path, &OperationContext::default()).unwrap();
        }
        let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
        let before: String = db
            .query_row(
                "SELECT content_hash FROM files WHERE path='lib.ts'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        drop(db);
        std::fs::write(dir.path().join("lib.ts"), "\nexport function run(){}").unwrap();
        let result = sync_file_with(dir.path(), "lib.ts", &OperationContext::default(), || {
            std::fs::write(
                dir.path().join("barrel.ts"),
                "export {missing as api} from './lib';",
            )
            .unwrap();
        });
        assert_eq!(result.unwrap_err().0, "STALE");
        let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT content_hash FROM files WHERE path='lib.ts'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            before
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM edges WHERE kind='calls'", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            1
        );
    }
    #[test]
    fn import_source_changed_after_resolution_rolls_back_target_update() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        std::fs::write(dir.path().join("lib.ts"), "export function run() {}").unwrap();
        sync_file(dir.path(), "lib.ts", &OperationContext::default()).unwrap();
        std::fs::write(
            dir.path().join("app.ts"),
            "import {run as execute} from './lib'; export function caller() { execute(); }",
        )
        .unwrap();
        sync_file(dir.path(), "app.ts", &OperationContext::default()).unwrap();
        let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
        let before: String = db
            .query_row(
                "SELECT content_hash FROM files WHERE path='lib.ts'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        drop(db);
        std::fs::write(dir.path().join("lib.ts"), "\nexport function run() {}").unwrap();
        let result = sync_file_with(dir.path(), "lib.ts", &OperationContext::default(), || {
            std::fs::write(dir.path().join("app.ts"), "import {run as execute} from './lib'; export function caller(execute: () => void) { execute(); }").unwrap();
        });
        assert_eq!(result.unwrap_err().0, "STALE");
        let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT content_hash FROM files WHERE path='lib.ts'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            before
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM edges WHERE kind='calls'", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            1
        );
    }
}
