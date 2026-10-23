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

#[path = "index_store/sql_objects.rs"]
pub(super) mod sql_objects;

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

#[path = "index_store/workspace.rs"]
mod workspace;

#[path = "index_store/changed_paths.rs"]
mod changed_paths;

#[cfg(windows)]
#[path = "index_store/transaction.rs"]
mod transaction;

#[cfg(windows)]
#[path = "index_store/file.rs"]
mod file;

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
    let mut port = changed_paths::NativeChangedPaths::new(root)?;
    harness_application::indexing::changed_paths::sync_changed_paths(&mut port, paths, context)
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
    let mut port = workspace::NativeWorkspace::new(root)?;
    harness_application::indexing::workspace::sync_workspace(&mut port, context, max_files, rebuild)
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
    if Path::new(relative)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("sql"))
    {
        return sync_changed_paths(
            root,
            &BTreeSet::from([normalize_relative(relative)]),
            context,
        );
    }
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
        &publication::IncrementalSqliteInvalidator,
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
    invalidator: &publication::ReferenceInvalidator,
) -> Result<Value, Failure> {
    let mut job = file::NativeFile {
        root,
        relative: normalize_relative(relative),
        requested_config,
        before_recheck: Some(before_recheck),
        force,
        writer,
        prepared,
        invalidator,
        project: None,
        scope: None,
        source: None,
        language: None,
    };
    harness_application::indexing::file::sync_file(&mut job, context)
}

fn read_source(
    project: &Project,
    path: &str,
    context: &OperationContext,
) -> Result<Option<Vec<u8>>, Failure> {
    let _profile = sync_diagnostic::source_read(path);
    #[cfg(windows)]
    {
        project.index_source_bytes(path, context, index_sync::MAX_SOURCE_BYTES as u64)
    }
    #[cfg(not(windows))]
    read_source_unpinned(project, path, context)
}

#[cfg(not(windows))]
fn read_source_unpinned(
    project: &Project,
    path: &str,
    context: &OperationContext,
) -> Result<Option<Vec<u8>>, Failure> {
    context.check()?;
    let target = {
        let _timer = sync_diagnostic::operation("source_path_guard");
        project.target_optional(path)?
    };
    let metadata = {
        let _timer = sync_diagnostic::operation("source_metadata");
        std::fs::symlink_metadata(&target)
    };
    match metadata {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            sync_diagnostic::count("source_missing", 1);
            return Ok(None);
        }
        Err(_) => return Err(("IO", "Source metadata unavailable; absence not established")),
        Ok(metadata) if !metadata.is_file() => {
            return Err(("PATH", "Source is not a regular file"));
        }
        Ok(_) => {}
    }
    #[cfg(windows)]
    let _pins = {
        let _timer = sync_diagnostic::operation("source_pin_directory");
        super::edit_commit::pin_directory(project, target.parent().ok_or(INDEX)?)?
    };
    project
        .bytes_with_limit(path, context, index_sync::MAX_SOURCE_BYTES as u64)
        .map(Some)
}
