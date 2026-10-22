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
    schema_validation: Option<Value>,
    schema_check: Option<String>,
    error: Option<Value>,
}

thread_local! {
    static CURRENT: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub(super) fn begin(file: &str, database: &Path) {
    crate::adapters::index_extract::take_diagnostic();
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

pub(super) fn schema_issue(issue: Value) {
    CURRENT.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.schema_validation = Some(issue);
        }
    });
}

pub(super) fn schema_check(check: &'static str) {
    CURRENT.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.schema_check = Some(check.to_owned());
        }
    });
}

pub(super) fn schema(user_version: i64, application_id: i64, provider_schema_version: Option<i64>) {
    CURRENT.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.schema_version = Some(user_version);
            state.application_id = Some(application_id);
            state.provider_schema_version = provider_schema_version;
        }
    });
}

pub(super) fn capture(error: &Error) {
    CURRENT.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut()
            && state.error.is_none()
        {
            let (sqlite_code, primary_code, extended_code, message, busy) = match error {
                Error::SqliteFailure(code, message) => (
                    Some(code_name(code.code)),
                    Some(code.code as i32),
                    Some(code.extended_code),
                    message.clone(),
                    matches!(
                        code.code,
                        ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
                    ),
                ),
                _ => (None, None, None, None, false),
            };
            let original_message = message.unwrap_or_else(|| error.to_string());
            state.error = Some(json!({
                "errorType": format!("{error:?}").split(['(', '{']).next().unwrap_or("unknown"),
                "originalMessage": original_message,
                "debug": format!("{error:?}"),
                "sqliteCode": sqlite_code,
                "sqlitePrimaryCode": primary_code,
                "sqliteExtendedCode": extended_code,
                "stage": state.stage,
                "schemaCheck": state.schema_check,
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
                "schemaCheck": state.schema_check,
                "databasePath": state.database,
                "file": state.file,
                "schema": {
                    "userVersion": state.schema_version,
                    "applicationId": state.application_id,
                    "providerSchemaVersion": state.provider_schema_version
                },
                "providerVersion": env!("CARGO_PKG_VERSION"),
                "sqliteVersion": rusqlite::version(),
                "sqlite": state.error,
                "extraction": crate::adapters::index_extract::take_diagnostic(),
                "schemaValidation": state.schema_validation.or_else(|| {
                    state.schema_check.map(|check| json!({
                        "check": check,
                        "outcome": "failed before validation result was produced"
                    }))
                })
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

    #[test]
    fn retains_non_sqlite_rusqlite_errors_and_validation_check() {
        begin("src/app.js", Path::new(".codegraph/codegraph.db"));
        stage("validate_schema");
        schema_check("provider_schema_version_query");
        capture(&Error::InvalidColumnType(
            0,
            "version".to_owned(),
            rusqlite::types::Type::Null,
        ));
        let diagnostic = take("SYNC_INDEX", "schema validation failed").unwrap();
        assert_eq!(diagnostic["schemaCheck"], "provider_schema_version_query");
        assert_eq!(
            diagnostic["schemaValidation"]["check"],
            "provider_schema_version_query"
        );
        assert_eq!(diagnostic["sqlite"]["errorType"], "InvalidColumnType");
        assert!(
            diagnostic["sqlite"]["originalMessage"]
                .as_str()
                .unwrap()
                .contains("version")
        );
        assert_eq!(diagnostic["sqlite"]["sqliteCode"], Value::Null);
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
