use super::super::project::Project;
use super::{BUDGET, INDEX};
use harness_domain::{Failure, execution::OperationContext};
use rusqlite::Error;
use rusqlite::{Connection, OpenFlags, limits::Limit};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    path::PathBuf,
    time::{Duration, Instant},
};

pub(super) fn index_path(
    project: &Project,
    diagnostic: &RefCell<Value>,
) -> Result<PathBuf, Failure> {
    let path = project
        .target_optional(".codegraph/codegraph.db")
        .map_err(|_| {
            diagnostic.borrow_mut()["cause"] = json!("database_path_rejected");
            INDEX
        })?;
    let metadata = std::fs::metadata(&path).map_err(|error| {
        diagnostic.borrow_mut()["cause"] = json!(match error.kind() {
            std::io::ErrorKind::NotFound => "database_missing",
            std::io::ErrorKind::PermissionDenied => "permission_denied",
            _ => "metadata_unavailable",
        });
        INDEX
    })?;
    if !metadata.is_file() || metadata.len() > 512 * 1024 * 1024 {
        diagnostic.borrow_mut()["cause"] = json!("database_type_or_size_rejected");
        return Err(INDEX);
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        project
            .target_optional(&format!(".codegraph/codegraph.db{suffix}"))
            .map_err(|_| INDEX)?;
    }
    Ok(path)
}

pub(super) fn failure(
    error: Error,
    diagnostic: &RefCell<Value>,
    context: &OperationContext,
    started: Instant,
) -> Failure {
    let mut details = diagnostic.borrow_mut();
    if let Error::SqliteFailure(code, _) = &error {
        details["sqliteCode"] = json!(format!("{:?}", code.code));
        details["sqliteExtendedCode"] = json!(code.extended_code);
        details["cause"] = json!(match code.code {
            rusqlite::ErrorCode::DatabaseBusy => "database_busy",
            rusqlite::ErrorCode::DatabaseLocked => "database_locked",
            rusqlite::ErrorCode::DatabaseCorrupt => "database_corrupt",
            rusqlite::ErrorCode::NotADatabase => "not_a_database",
            rusqlite::ErrorCode::PermissionDenied | rusqlite::ErrorCode::CannotOpen => {
                "database_inaccessible"
            }
            _ => "sqlite_operation_failed",
        });
    } else {
        details["cause"] = json!("schema_or_value_contract_mismatch");
    }
    context.check().err().unwrap_or_else(|| {
        if started.elapsed() >= Duration::from_secs(5) {
            BUDGET
        } else {
            INDEX
        }
    })
}

pub(super) struct OpenIndex {
    pub(super) connection: Connection,
    pub(super) version: i64,
    pub(super) has_qualified: bool,
}

pub(super) fn open(
    path: &std::path::Path,
    context: &OperationContext,
    diagnostic: &RefCell<Value>,
    started: Instant,
) -> Result<OpenIndex, Failure> {
    let failed = |error| failure(error, diagnostic, context, started);
    diagnostic.borrow_mut()["stage"] = json!("sqlite_open");
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(failed)?;
    db.busy_timeout(Duration::from_millis(100))
        .map_err(failed)?;
    db.set_limit(Limit::SQLITE_LIMIT_LENGTH, 65536)
        .map_err(failed)?;
    db.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 8192)
        .map_err(failed)?;
    let cancel = context.clone();
    db.progress_handler(
        1000,
        Some(move || cancel.check().is_err() || started.elapsed() >= Duration::from_secs(5)),
    )
    .map_err(failed)?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-4096; BEGIN;")
        .map_err(failed)?;
    // Refuse views and virtual-table substitutions; this adapter supports only
    // the installed CodeGraph table contract, not arbitrary SQLite databases.
    diagnostic.borrow_mut()["stage"] = json!("schema_validation");
    let tables: i64 = db
        .query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('schema_versions','nodes','edges') AND type='table' AND upper(sql) LIKE 'CREATE TABLE%'", [], |r| r.get(0))
        .map_err(failed)?;
    if tables != 3 {
        diagnostic.borrow_mut()["cause"] = json!("required_tables_missing_or_unsupported");
        return Err(INDEX);
    }
    let version: i64 = db
        .query_row("SELECT max(version) FROM schema_versions", [], |r| r.get(0))
        .map_err(failed)?;
    diagnostic.borrow_mut()["actualSchemaVersion"] = json!(version);
    if !(1..=11).contains(&version) {
        diagnostic.borrow_mut()["cause"] = json!("unsupported_schema_version");
        return Err(INDEX);
    }
    let has_qualified: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('nodes') WHERE name='qualified_name')",
            [],
            |r| r.get(0),
        )
        .map_err(failed)?;
    Ok(OpenIndex {
        connection: db,
        version,
        has_qualified,
    })
}
