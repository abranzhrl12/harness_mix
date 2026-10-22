use super::{
    ResolvedSqlObject, RoutineCatalog, operation_failed, operation_failed_with_server_diagnostic,
    relation_response, routine_response,
};
use serde_json::{Value, json};

#[test]
fn failures_keep_the_public_sqlserver_response_shape() {
    let output = operation_failed(("TIMEOUT", "operation deadline exceeded"));
    assert_eq!(output["ok"], false);
    assert_eq!(output["engine"], "sqlserver");
    assert_eq!(output["timedOut"], true);
    assert_eq!(output["cancelled"], false);
    assert_eq!(output["diagnostics"][0]["code"], "TIMEOUT");
    assert_eq!(output["connectionDisposition"], "closed_if_opened");
    assert_eq!(output["serverCancellationConfirmed"], false);
}

#[test]
fn sanitized_server_diagnostic_replaces_only_the_public_message() {
    let diagnostic = json!({"number": 208, "message": "SQL Server 208: Invalid object name 'app.T'", "messageSanitized": true});
    let output = operation_failed_with_server_diagnostic(
        ("SQL_INVALID_OBJECT", "generic message"),
        Some(&diagnostic),
    );
    assert_eq!(output["diagnostics"][0]["message"], diagnostic["message"]);
    assert_eq!(output["serverError"], diagnostic);
    assert!(!output.to_string().contains("Password="));
}

#[test]
fn describe_presenters_preserve_resolution_and_truncation_fields() {
    let resolved = ResolvedSqlObject {
        schema: "app".into(),
        name: "refresh]cache".into(),
        object_id: 42,
        kind: "SQL_STORED_PROCEDURE".into(),
    };
    let routine = routine_response(
        "refresh]cache",
        &resolved,
        RoutineCatalog {
            definition: Value::Null,
            definition_available: false,
            parameters: vec![json!({"name": "@force"})],
            definition_truncated: false,
            parameters_truncated: true,
        },
    );
    assert_eq!(routine["resolvedName"], "[app].[refresh]]cache]");
    assert_eq!(routine["parametersComplete"], false);
    assert_eq!(routine["truncated"], true);
    assert_eq!(
        routine["diagnostics"][0]["code"],
        "SQL_DEFINITION_UNAVAILABLE"
    );

    let relation = relation_response("items", &resolved, vec![json!({"column": "id"})], true);
    assert_eq!(relation["kind"], "relation");
    assert_eq!(relation["columns"][0]["column"], "id");
    assert_eq!(relation["truncated"], true);
}
