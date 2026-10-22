use rusqlite::{Error, ErrorCode};
use serde_json::{Value, json};
use std::{cell::RefCell, path::Path};

#[derive(Default)]
struct State {
    file: String,
    database: String,
    stage: &'static str,
    schema_version: Option<i64>,
    application_id: Option<i64>,
    provider_schema_version: Option<i64>,
    error: Option<Value>,
}

thread_local! {
    static CURRENT: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub(super) fn begin(file: &str, database: &Path) {
    CURRENT.with(|slot| {
        *slot.borrow_mut() = Some(State {
            file: file.to_owned(),
            database: database.display().to_string(),
            stage: "open_database",
            ..State::default()
        });
    });
}

pub(super) fn stage(stage: &'static str) {
    CURRENT.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.stage = stage;
        }
    });
}

pub(super) fn schema(
    user_version: i64,
    application_id: i64,
    provider_schema_version: Option<i64>,
) {
    CURRENT.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.schema_version = Some(user_version);
            state.application_id = Some(application_id);
            state.provider_schema_version = provider_schema_version;
        }
    });
}

pub(super) fn capture(error: &Error) {
    let Error::SqliteFailure(code, message) = error else {
        return;
    };
    CURRENT.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut()
            && state.error.is_none()
        {
            let busy = matches!(code.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked);
            state.error = Some(json!({
                "sqliteCode": code_name(code.code),
                "sqlitePrimaryCode": code.code as i32,
                "sqliteExtendedCode": code.extended_code,
                "originalMessage": message.as_deref().unwrap_or("SQLite supplied no message"),
                "stage": state.stage,
                "databasePath": state.database,
                "file": state.file,
                "schema": {
                    "userVersion": state.schema_version,
                    "applicationId": state.application_id,
                    "providerSchemaVersion": state.provider_schema_version
                },
                "providerVersion": env!("CARGO_PKG_VERSION"),
                "sqliteVersion": rusqlite::version(),
                "competingWriterDetected": busy,
                "competingProcessId": null,
            }));
        }
    });
}

pub(super) fn take(failure_code: &str, failure_message: &str) -> Option<Value> {
    CURRENT.with(|slot| {
        slot.borrow_mut().take().map(|state| {
            json!({
                "failureCode": failure_code,
                "failureMessage": failure_message,
                "stage": state.stage,
                "databasePath": state.database,
                "file": state.file,
                "schema": {
                    "userVersion": state.schema_version,
                    "applicationId": state.application_id,
                    "providerSchemaVersion": state.provider_schema_version
                },
                "providerVersion": env!("CARGO_PKG_VERSION"),
                "sqliteVersion": rusqlite::version(),
                "sqlite": state.error
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_context_for_schema_failures_without_sqlite_exception() {
        begin("src/app.js", Path::new(".codegraph/codegraph.db"));
        stage("validate_schema");
        schema(0, 42, Some(7));
        let diagnostic = take("SYNC_SCHEMA", "unsupported schema").unwrap();
        assert_eq!(diagnostic["failureCode"], "SYNC_SCHEMA");
        assert_eq!(diagnostic["stage"], "validate_schema");
        assert_eq!(diagnostic["file"], "src/app.js");
        assert_eq!(diagnostic["databasePath"], ".codegraph/codegraph.db");
        assert_eq!(diagnostic["schema"]["providerSchemaVersion"], 7);
        assert!(diagnostic["sqlite"].is_null());
    }
}

fn code_name(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::DatabaseBusy => "SQLITE_BUSY",
        ErrorCode::DatabaseLocked => "SQLITE_LOCKED",
        ErrorCode::DatabaseCorrupt => "SQLITE_CORRUPT",
        ErrorCode::NotADatabase => "SQLITE_NOTADB",
        ErrorCode::SchemaChanged => "SQLITE_SCHEMA",
        ErrorCode::ConstraintViolation => "SQLITE_CONSTRAINT",
        ErrorCode::ReadOnly => "SQLITE_READONLY",
        ErrorCode::DiskFull => "SQLITE_FULL",
        ErrorCode::CannotOpen => "SQLITE_CANTOPEN",
        ErrorCode::SystemIoFailure => "SQLITE_IOERR",
        ErrorCode::PermissionDenied => "SQLITE_PERM",
        _ => "SQLITE_ERROR",
    }
}
