use harness_domain::Failure;
use serde_json::{Value, json};

/// Classify errors without returning raw driver text or connection details.
pub(super) fn sql_error(error: tiberius::error::Error) -> Failure {
    match error.code() {
        Some(207) => ("SQL_INVALID_COLUMN", "SQL Server 207: Invalid column name"),
        Some(208) => ("SQL_INVALID_OBJECT", "SQL Server 208: Invalid object name"),
        Some(209) => (
            "SQL_AMBIGUOUS_COLUMN",
            "SQL Server 209: Ambiguous column name",
        ),
        Some(2812) => (
            "SQL_PROCEDURE_NOT_FOUND",
            "SQL Server 2812: Stored procedure not found",
        ),
        Some(4104) => (
            "SQL_IDENTIFIER_BINDING",
            "SQL Server 4104: Multi-part identifier could not be bound",
        ),
        Some(102 | 156) => (
            "SQL_SYNTAX",
            "SQL Server syntax error; check identifiers and SQL keywords",
        ),
        Some(18456 | 18452) => ("SQL_AUTH", "SQL Server authentication failed"),
        Some(229 | 230 | 262 | 297 | 916) => {
            ("SQL_PERMISSION", "SQL Server login lacks permission")
        }
        Some(1222) => ("SQL_LOCK_TIMEOUT", "SQL Server lock wait timed out"),
        Some(1205) => (
            "SQL_DEADLOCK",
            "SQL Server selected this operation as deadlock victim",
        ),
        _ => match error {
            tiberius::error::Error::Tls(_) => (
                "SQL_TLS",
                "SQL Server TLS negotiation or certificate validation failed",
            ),
            tiberius::error::Error::Io { .. } => ("SQL_CONNECTION", "SQL Server transport failed"),
            _ => (
                "SQL_QUERY",
                "SQL Server operation failed; driver details withheld",
            ),
        },
    }
}

/// Only recognized schema errors may echo a single, bounded SQL identifier.
/// Reconstruct the message rather than trying to redact arbitrary server text.
pub(super) fn safe_schema_message(number: u32, message: &str) -> Option<String> {
    let (prefix, suffix) = match number {
        207 => ("Invalid column name '", "'."),
        208 => ("Invalid object name '", "'."),
        209 => ("Ambiguous column name '", "'."),
        2812 => ("Could not find stored procedure '", "'."),
        4104 => ("The multi-part identifier \"", "\" could not be bound."),
        _ => return None,
    };
    let identifier = message.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if identifier.is_empty()
        || identifier.len() > 256
        || !identifier
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '[' | ']' | '@' | '#' | '$'))
    {
        return None;
    }
    Some(format!("SQL Server {number}: {prefix}{identifier}{suffix}"))
}

pub(super) fn server_diagnostic(error: &tiberius::error::Error) -> Option<Value> {
    let tiberius::error::Error::Server(server) = error else {
        return None;
    };
    let number = server.code();
    // Never expose authentication, conversion, duplicate-value or user-thrown text.
    let message = safe_schema_message(number, server.message());
    Some(json!({"number": number, "message": message, "messageSanitized": true}))
}

pub(super) fn operation_failed((code, message): Failure) -> Value {
    json!({"ok":false,"engine":"sqlserver","timedOut":crate::adapters::sql_policy::timed_out(code),"cancelled":code=="CANCELLED",
        "diagnostics":[{"code":code,"message":message}],"connectionReused":false,
        "connectionDisposition":"closed_if_opened","cancellationRequestSent":false,"serverCancellationConfirmed":false})
}

/// Keep the public diagnostic consistent with the bounded server diagnostic.
pub(super) fn operation_failed_with_server_diagnostic(
    error: Failure,
    diagnostic: Option<&Value>,
) -> Value {
    let mut output = operation_failed(error);
    if let Some(diagnostic) = diagnostic {
        if let Some(message) = diagnostic["message"].as_str() {
            output["diagnostics"][0]["message"] = json!(message);
        }
        output["serverError"] = diagnostic.clone();
    }
    output
}
