//! First stage of the CodeGraph synchronizer port: bounded, read-only planning.
//!
//! The caller supplies an inventory from the SAME scope as the indexed files.
//! `complete` must remain false after any exclusion, scan error or budget hit.
//! This module neither extracts symbols nor publishes hashes or graph mutations.
#[cfg(test)]
use harness_domain::hash;
use harness_domain::{Failure, execution::OperationContext};
use rusqlite::Connection;
use std::collections::BTreeMap;

fn sql_failure(error: rusqlite::Error, failure: Failure) -> Failure {
    // Preserve the original SQLite cause even when the public failure is scoped
    // to planning rather than publication.
    let _ = super::index_store::sql::<()>(Err(error));
    failure
}

/// Executable dry-run stage. It checks every provider-tracked file and discovers
/// new Rust files, but deliberately cannot claim the full CodeGraph scan scope.
pub fn plan_workspace(
    root: &std::path::Path,
    context: &OperationContext,
) -> Result<serde_json::Value, Failure> {
    plan_workspace_inner(root, context, true)
}

pub(super) fn plan_workspace_for_sync(
    root: &std::path::Path,
    context: &OperationContext,
) -> Result<serde_json::Value, Failure> {
    plan_workspace_inner(root, context, false)
}

fn plan_workspace_inner(
    root: &std::path::Path,
    context: &OperationContext,
    preview: bool,
) -> Result<serde_json::Value, Failure> {
    use rusqlite::{OpenFlags, limits::Limit};
    use std::time::Duration;
    context.check()?;
    context.report_progress("planning", 0, None, None);
    let project = super::project::Project::new(root)
        .map_err(|_| ("PATH", "Workspace is not an accessible directory"))?;
    super::index_store::begin_plan_diagnostic(&project.root.join(".codegraph/codegraph.db"));
    let index_error = (
        "INDEX",
        "CodeGraph index unavailable, unsupported, busy or invalid",
    );
    let path = project
        .target(".codegraph/codegraph.db")
        .map_err(|_| index_error)?;
    let metadata = std::fs::metadata(&path).map_err(|_| index_error)?;
    if !metadata.is_file() || metadata.len() > 512 * 1024 * 1024 {
        return Err(index_error);
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        project
            .target_optional(&format!(".codegraph/codegraph.db{suffix}"))
            .map_err(|_| index_error)?;
    }
    #[cfg(windows)]
    let _pins = super::edit_commit::pin_directory(&project, path.parent().ok_or(index_error)?)?;
    #[cfg(windows)]
    let _database_pin = super::index_lock::open_database_pin(&path, context).map_err(|error| {
        if let Err(cancelled) = context.check() {
            return cancelled;
        }
        if error.kind() == std::io::ErrorKind::TimedOut {
            (
                "INDEX_BUSY",
                "CodeGraph index remained locked until the wait timed out",
            )
        } else {
            index_error
        }
    })?;
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|error| sql_failure(error, index_error))?;
    db.busy_timeout(Duration::from_millis(super::INDEX_BUSY_TIMEOUT_MS))
        .map_err(|error| sql_failure(error, index_error))?;
    db.set_limit(Limit::SQLITE_LIMIT_LENGTH, 131072)
        .map_err(|error| sql_failure(error, index_error))?;
    db.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 8192)
        .map_err(|error| sql_failure(error, index_error))?;
    let cancel = context.clone();
    db.progress_handler(1000, Some(move || cancel.check().is_err()))
        .map_err(|error| sql_failure(error, index_error))?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-4096; BEGIN;").map_err(|error| sql_failure(error, index_error))?;
    let run = || {
        let ordinary: i64 = db.query_row("SELECT count(*) FROM sqlite_schema WHERE name='schema_versions' AND type='table' AND upper(sql) LIKE 'CREATE TABLE%'", [], |row| row.get(0)).map_err(|error| sql_failure(error, index_error))?;
        if ordinary != 1 {
            return Err(index_error);
        }
        let version: i64 = db
            .query_row("SELECT max(version) FROM schema_versions", [], |row| {
                row.get(0)
            })
            .map_err(|error| sql_failure(error, index_error))?;
        if !(1..=11).contains(&version) {
            return Err(index_error);
        }
        let (result, inventory, extraction) =
            harness_application::indexing::planning::plan_workspace(
                &mut NativePlanner {
                    db: &db,
                    project: &project,
                },
                preview,
                context,
            )?;
        Ok(serde_json::json!({
            "mode":"plan_only", "format":"codegraph-sqlite-readonly-v1", "schemaVersion":version,
            "plan":result, "scope":inventory.scope,"inventory":inventory, "extractionPreview":extraction, "writesPerformed":false, "synchronized":false,
            "coverage":{
                "trackedLanguages":"all_nonrestricted", "newFileLanguages":codegraph_native::LANGUAGES, "fullProviderScope":false,
                "providerConfigApplied":inventory.scope["providerConfigApplied"], "trackedFilesCheckedOutsideDiscoveryIgnores":true,
                "deletionsRequireExplicitAbsenceOrConfigExclusion":true,
                "limitations":["Only bounded include/exclude config is supported; extension overrides and includeIgnored are refused", "Native-language new-file discovery, including Vue and HTML, uses local gitignore rules and conservative directory exclusions", "Not an atomic source snapshot; observations must be revalidated before publication", "Extraction preview does not resolve or publish the graph"]
            }
        }))
    };
    let result = run();
    // Closing also rolls back on all error paths; never mask the original error.
    context.check()?;
    result
}

pub use harness_application::indexing::planning::{
    MAX_FILES, MAX_SOURCE_BYTES, Observation, Plan, source_hash,
};
use harness_application::indexing::planning::{valid_hash, valid_path};

/// Reads only provider `files(path, content_hash, errors)` rows. The caller owns
/// the connection/snapshot and must open real provider files read-only. No schema
/// creation, migrations, SQL supplied by users, extraction, or writes occur here.
pub fn plan(
    db: &Connection,
    observed: &BTreeMap<String, Observation>,
    complete: bool,
    context: &OperationContext,
) -> Result<Plan, Failure> {
    context.check()?;
    if observed.len() > MAX_FILES {
        return Err(("SYNC_LIMIT", "Source inventory exceeds the file budget"));
    }
    for (path, observation) in observed {
        context.check()?;
        if !valid_path(path) {
            return Err((
                "SYNC_CONTRACT",
                "Inventory contains an invalid relative path",
            ));
        }
        if let Observation::Content(digest) = observation
            && !valid_hash(digest)
        {
            return Err(("SYNC_CONTRACT", "Inventory contains an invalid source hash"));
        }
    }
    let indexed = read_indexed(db, context)?;
    harness_application::indexing::planning::classify(&indexed, observed, complete, context)
}

fn read_indexed(
    db: &Connection,
    context: &OperationContext,
) -> Result<BTreeMap<String, (String, bool)>, Failure> {
    context.check()?;
    let ordinary: i64 = db.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE name='files' AND type='table' AND upper(sql) LIKE 'CREATE TABLE%'",
        [], |row| row.get(0),
    ).map_err(|error| sql_failure(error, ("SYNC_CONTRACT", "Cannot inspect indexed files metadata")))?;
    if ordinary != 1 {
        return Err(("SYNC_CONTRACT", "Expected the CodeGraph files table"));
    }
    // Fixed limit bounds legacy/malformed databases even without a primary key.
    let mut statement = db
        .prepare(
            "SELECT path, content_hash, errors FROM files ORDER BY path COLLATE BINARY LIMIT 20001",
        )
        .map_err(|error| {
            sql_failure(
                error,
                ("SYNC_CONTRACT", "Incompatible CodeGraph files columns"),
            )
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|error| sql_failure(error, ("SYNC_CONTRACT", "Cannot read indexed files")))?;
    let mut indexed = BTreeMap::new();
    for row in rows {
        context.check()?;
        let (path, digest, errors) = row.map_err(|error| {
            sql_failure(error, ("SYNC_CONTRACT", "Invalid indexed file metadata"))
        })?;
        if indexed.len() == MAX_FILES {
            return Err(("SYNC_LIMIT", "Index exceeds the file budget"));
        }
        if !valid_path(&path)
            || !valid_hash(&digest)
            || errors.as_ref().is_some_and(|v| v.len() > 65536)
        {
            return Err(("SYNC_CONTRACT", "Invalid indexed file metadata"));
        }
        let clean = errors.is_none_or(|value| {
            serde_json::from_str::<serde_json::Value>(&value)
                .is_ok_and(|v| v.as_array().is_some_and(Vec::is_empty))
        });
        if indexed.insert(path, (digest, clean)).is_some() {
            return Err(("SYNC_CONTRACT", "Duplicate indexed file path"));
        }
    }
    Ok(indexed)
}

struct NativePlanner<'a> {
    db: &'a Connection,
    project: &'a super::project::Project,
}
impl harness_application::ports::indexing::PlanningPort for NativePlanner<'_> {
    fn indexed(
        &mut self,
        context: &OperationContext,
    ) -> Result<BTreeMap<String, (String, bool)>, Failure> {
        read_indexed(self.db, context)
    }
    fn inventory(
        &mut self,
        tracked: &[String],
        context: &OperationContext,
    ) -> Result<super::index_inventory::Inventory, Failure> {
        super::index_inventory::scan(self.project, tracked, context)
    }
    fn restricted(&self, path: &str) -> bool {
        super::index_inventory::restricted(path)
    }
    fn preview(
        &mut self,
        plan: &Plan,
        inventory: &super::index_inventory::Inventory,
        context: &OperationContext,
    ) -> Result<serde_json::Value, Failure> {
        super::index_extract::preview(self.project, plan, &inventory.observations, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE files(path TEXT PRIMARY KEY, content_hash TEXT NOT NULL, errors TEXT);",
        )
        .unwrap();
        for (path, errors) in [
            ("a.rs", None),
            ("b.cs", None),
            ("c.js", Some("[]")),
            ("d.py", Some("[{\"message\":\"failed\"}]")),
        ] {
            db.execute(
                "INSERT INTO files VALUES (?1, ?2, ?3)",
                rusqlite::params![path, hash(b"old"), errors],
            )
            .unwrap();
        }
        db
    }

    #[test]
    fn classifies_incremental_work_without_mutating_provider() {
        let db = fixture();
        db.execute_batch("PRAGMA query_only=ON").unwrap();
        let observations = BTreeMap::from([
            ("a.rs".into(), Observation::Content(hash(b"new"))),
            ("c.js".into(), Observation::Content(hash(b"old"))),
            ("d.py".into(), Observation::Content(hash(b"old"))),
            ("new.rs".into(), Observation::Content(hash(b"added"))),
        ]);
        let result = plan(&db, &observations, true, &OperationContext::default()).unwrap();
        assert_eq!(result.added, ["new.rs"]);
        assert_eq!(result.modified, ["a.rs"]);
        assert_eq!(result.removed, ["b.cs"]);
        assert_eq!(result.unchanged, ["c.js"]);
        assert_eq!(result.retry_extraction, ["d.py"]);
        assert!(!result.synchronized);
        assert_eq!(db.total_changes(), 4);
    }

    #[test]
    fn incomplete_scan_and_unreadable_files_never_imply_deletion() {
        let db = fixture();
        let observed = BTreeMap::from([
            ("a.rs".into(), Observation::Unchecked),
            ("b.cs".into(), Observation::Missing),
        ]);
        let result = plan(&db, &observed, false, &OperationContext::default()).unwrap();
        assert_eq!(result.removed, ["b.cs"]);
        assert_eq!(result.unchecked, ["a.rs", "c.js", "d.py"]);
        let result = plan(
            &db,
            &BTreeMap::from([("a.rs".into(), Observation::Unchecked)]),
            true,
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(result.unchecked, ["a.rs"]);
        assert!(!result.removed.contains(&"a.rs".into()));
    }

    #[test]
    fn rejects_invalid_metadata_paths_views_and_hashes() {
        let db = fixture();
        for path in [
            "../escape",
            "a\\b.rs",
            "/absolute",
            "C:/file",
            "a//b",
            "a/./b",
        ] {
            let observed = BTreeMap::from([(path.into(), Observation::Unchecked)]);
            assert_eq!(
                plan(&db, &observed, true, &OperationContext::default())
                    .unwrap_err()
                    .0,
                "SYNC_CONTRACT"
            );
        }
        db.execute(
            "UPDATE files SET content_hash='invalid' WHERE path='a.rs'",
            [],
        )
        .unwrap();
        assert!(plan(&db, &BTreeMap::new(), true, &OperationContext::default()).is_err());
        db.execute_batch("DROP TABLE files; CREATE VIEW files AS SELECT 'x' AS path, 'y' AS content_hash, NULL AS errors;").unwrap();
        assert!(plan(&db, &BTreeMap::new(), true, &OperationContext::default()).is_err());
    }

    #[test]
    fn cancellation_and_inventory_budget_abort_without_partial_plan() {
        let db = fixture();
        let context = OperationContext::default();
        context.cancel();
        assert_eq!(
            plan(&db, &BTreeMap::new(), true, &context).unwrap_err().0,
            "CANCELLED"
        );
        let observed = (0..=MAX_FILES)
            .map(|i| (format!("{i}.rs"), Observation::Unchecked))
            .collect();
        assert_eq!(
            plan(&db, &observed, true, &OperationContext::default())
                .unwrap_err()
                .0,
            "SYNC_LIMIT"
        );
    }

    #[test]
    fn malformed_extraction_status_is_retried_not_treated_as_clean() {
        let db = fixture();
        let observed = BTreeMap::from([("a.rs".into(), Observation::Content(hash(b"old")))]);
        for errors in ["not json", "{}", "null", "[1]"] {
            db.execute("UPDATE files SET errors=?1 WHERE path='a.rs'", [errors])
                .unwrap();
            let result = plan(&db, &observed, false, &OperationContext::default()).unwrap();
            assert_eq!(result.retry_extraction, ["a.rs"]);
            assert!(result.unchanged.is_empty());
        }
        db.execute(
            "UPDATE files SET errors=NULL, content_hash=upper(content_hash) WHERE path='a.rs'",
            [],
        )
        .unwrap();
        let result = plan(&db, &observed, false, &OperationContext::default()).unwrap();
        assert_eq!(result.unchanged, ["a.rs"]);
        assert!(!result.synchronized);
    }

    #[test]
    fn hashing_matches_codegraph_text_and_rejects_skipped_sources() {
        for text in ["", "fn main() {}\r\n", "á😀\n", "\u{feff}source"] {
            assert_eq!(source_hash(text.as_bytes()).unwrap(), hash(text.as_bytes()));
        }
        assert_eq!(source_hash(&[0xff]).unwrap(), hash(&[0xff]));
        assert_eq!(source_hash(&[0x81]).unwrap_err().0, "SYNC_UNCHECKED");
        assert!(source_hash(&vec![b'x'; MAX_SOURCE_BYTES]).is_ok());
        assert_eq!(
            source_hash(&vec![b'x'; MAX_SOURCE_BYTES + 1])
                .unwrap_err()
                .0,
            "SYNC_LIMIT"
        );
    }
}
