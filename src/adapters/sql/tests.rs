use super::{
    connection::block_on_safe,
    diagnostics::{
        operation_failed, operation_failed_with_server_diagnostic, safe_schema_message,
        server_diagnostic, sql_error,
    },
    operations::{escape_like, inject_top},
};
use serde_json::json;
use std::time::Duration;
#[test]
fn literal_like_wildcards_are_escaped_without_replacement_templates() {
    assert_eq!(escape_like("a%_~[é"), "a~%~_~~~[é");
}
#[test]
fn top_is_injected_only_when_a_plain_select_lacks_one() {
    for (sql, expected) in [
        ("SELECT 1", "SELECT TOP (50) 1"),
        ("select*from t", "select TOP (50)*from t"),
        (
            "select distinct name from t",
            "select distinct TOP (50) name from t",
        ),
        (
            "-- why\r\nselect /* c */ * from t",
            "-- why\r\nselect TOP (50) /* c */ * from t",
        ),
        ("select top 10 * from t", "select top 10 * from t"),
        ("select TOP (10) * from t", "select TOP (10) * from t"),
        (
            "with c as (select 1) select * from c",
            "with c as (select 1) select * from c",
        ),
        ("update t set a = 1", "update t set a = 1"),
    ] {
        assert_eq!(inject_top(sql, 50), expected, "{sql}");
    }
}
#[test]
fn driver_error_text_is_never_exposed() {
    assert_eq!(
        sql_error(tiberius::error::Error::Tls(
            "private connection details".into()
        ))
        .0,
        "SQL_TLS"
    );
    let failure = sql_error(tiberius::error::Error::Protocol(
        "SELECT private value".into(),
    ));
    assert!(!failure.1.contains("private"));
    let output = operation_failed(("TIMEOUT", "operation deadline exceeded"));
    assert_eq!(output["timedOut"], true);
    assert_eq!(output["serverCancellationConfirmed"], false);
}
#[test]
fn schema_errors_echo_only_allowlisted_identifiers() {
    assert_eq!(
        safe_schema_message(207, "Invalid column name 'tEmpresaRuc'.").as_deref(),
        Some("SQL Server 207: Invalid column name 'tEmpresaRuc'.")
    );
    assert!(safe_schema_message(208, "Invalid object name 'dbo.MEMPRESA'.").is_some());
    assert!(
        safe_schema_message(
            4104,
            "The multi-part identifier \"a.Id\" could not be bound."
        )
        .is_some()
    );
    for message in [
        "Invalid column name 'Password=secret'.",
        "Invalid column name 'valid'.\nUser ID=sa;Password=secret",
        "Invalid column name 'abc\nsecret'.",
        "Invalid column name 'SELECT private value'.",
        "Invalid column name 'x';Password=secret'.",
        "Invalid column name ''.",
    ] {
        assert!(safe_schema_message(207, message).is_none());
    }
    assert!(
        safe_schema_message(207, &format!("Invalid column name '{}'.", "x".repeat(257))).is_none()
    );
    assert!(safe_schema_message(18456, "Login failed for user 'secret'.").is_none());
    assert!(
        safe_schema_message(245, "Conversion failed when converting 'private value'.").is_none()
    );
    assert!(safe_schema_message(50000, "Invalid column name 'secret'.").is_none());
    assert!(
        server_diagnostic(&tiberius::error::Error::Protocol("Password=secret".into())).is_none()
    );
}
#[test]
fn invalid_column_is_visible_in_the_public_failure_without_raw_server_text() {
    for (raw, expected) in [
        (
            "Invalid column name 'tEmpresaRuc'.",
            "SQL Server 207: Invalid column name 'tEmpresaRuc'.",
        ),
        (
            "Invalid column name 'x';Password=secret'.",
            "SQL Server 207: Invalid column name",
        ),
    ] {
        let diagnostic = json!({
            "number": 207,
            "message": safe_schema_message(207, raw),
            "messageSanitized": true
        });
        let output = operation_failed_with_server_diagnostic(
            ("SQL_INVALID_COLUMN", "SQL Server 207: Invalid column name"),
            Some(&diagnostic),
        );
        assert_eq!(output["diagnostics"][0]["message"], expected);
        assert_eq!(output["diagnostics"][0]["code"], "SQL_INVALID_COLUMN");
        assert!(!output.to_string().contains("Password=secret"));
    }
}
#[tokio::test]
async fn sync_bridge_handles_current_thread_runtime() {
    assert_eq!(
        block_on_safe(async {
            tokio::time::sleep(Duration::from_millis(1)).await;
            42
        }),
        42
    );
}
