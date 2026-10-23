use rusqlite::{Error, ErrorCode};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap, path::Path, time::Instant};

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
    phase_started: Option<Instant>,
    phase_micros: BTreeMap<&'static str, u128>,
}

pub(super) use crate::adapters::index_profile::{Timer as ResolverTimer, operation, count, source_read};

thread_local! {
    static CURRENT: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub(super) fn profile_active() -> bool {
    CURRENT.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|state| state.phase_started.is_some())
    })
}

pub(super) fn resolver_timer(phase: &'static str) -> ResolverTimer {
    operation(phase)
}

pub(super) fn profile_metrics() -> Option<Value> {
    crate::adapters::index_profile::metrics()
}

pub(super) fn begin(file: &str, database: &Path) {
    crate::adapters::index_extract::take_diagnostic();
    let enabled = std::env::var_os("HARNESS_INDEX_PROFILE").is_some_and(|value| value == "1");
    crate::adapters::index_profile::begin(enabled);
    CURRENT.with(|slot| {
        *slot.borrow_mut() = Some(State {
            file: file.to_owned(),
            database: database.display().to_string(),
            stage: "open_database",
            phase_started: enabled.then(Instant::now),
            ..State::default()
        });
    });
}

pub(super) fn stage(stage: &'static str) {
    crate::adapters::index_profile::stage(stage);
    CURRENT.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            if let Some(started) = state.phase_started.as_mut() {
                *state.phase_micros.entry(state.stage).or_default() +=
                    started.elapsed().as_micros();
                *started = Instant::now();
            }
            state.stage = stage;
        }
    });
}

pub(super) fn finish_profile() -> Option<Value> {
    CURRENT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let state = slot.as_mut()?;
        let started = state.phase_started.take()?;
        *state.phase_micros.entry(state.stage).or_default() += started.elapsed().as_micros();
        crate::adapters::index_profile::finish();
        for (name, micros) in crate::adapters::index_profile::resolver_micros() {
            *state.phase_micros.entry(name).or_default() += micros;
        }
        Some(json!(state.phase_micros))
    })
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
    let profile = finish_profile();
    let metrics = profile_metrics();
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
                "phaseTimingsMicros": profile,
                "profileMetrics": metrics,
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
#[path = "sync_diagnostic_tests.rs"]
mod tests;

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
