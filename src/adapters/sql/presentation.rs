//! Pure JSON presentation for SQL Server operation results and failures.
use harness_domain::Failure;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedSqlObject {
    pub(super) schema: String,
    pub(super) name: String,
    pub(super) object_id: i32,
    pub(super) kind: String,
}

pub(super) struct RoutineCatalog {
    pub(super) definition: Value,
    pub(super) definition_available: bool,
    pub(super) parameters: Vec<Value>,
    pub(super) definition_truncated: bool,
    pub(super) parameters_truncated: bool,
}

pub(super) fn quote_sql_identifier(identifier: &str) -> String {
    format!("[{}]", identifier.replace(']', "]]"))
}

pub(super) fn routine_definition_diagnostic(definition: Option<&str>) -> Vec<Value> {
    if definition.is_some() {
        return Vec::new();
    }
    vec![json!({
        "code": "SQL_DEFINITION_UNAVAILABLE",
        "message": "The routine is visible in the catalog, but its definition is unavailable. VIEW DEFINITION permission may be missing, the module may be encrypted, or SQL Server may have another metadata visibility restriction; this result does not identify the cause."
    })]
}

pub(super) fn routine_response(
    name: &str,
    resolved: &ResolvedSqlObject,
    catalog: RoutineCatalog,
) -> Value {
    let diagnostics = routine_definition_diagnostic(catalog.definition_available.then_some(""));
    let mut response = json!({
        "engine": "sqlserver",
        "name": name,
        "resolvedName": format!("{}.{}", quote_sql_identifier(&resolved.schema), quote_sql_identifier(&resolved.name)),
        "schema": resolved.schema,
        "objectId": resolved.object_id,
        "objectType": resolved.kind,
        "kind": "routine",
        "definition": catalog.definition,
        "parameters": catalog.parameters,
        "parameterMetadataSource": "sys.parameters",
        "parametersComplete": !catalog.parameters_truncated,
        "truncated": catalog.definition_truncated || catalog.parameters_truncated
    });
    if !diagnostics.is_empty() {
        response["diagnostics"] = json!(diagnostics);
    }
    response
}

pub(super) fn relation_response(
    name: &str,
    resolved: &ResolvedSqlObject,
    columns: Vec<Value>,
    truncated: bool,
) -> Value {
    json!({
        "engine": "sqlserver",
        "name": name,
        "resolvedName": format!("{}.{}", quote_sql_identifier(&resolved.schema), quote_sql_identifier(&resolved.name)),
        "schema": resolved.schema,
        "objectId": resolved.object_id,
        "objectType": resolved.kind,
        "kind": "relation",
        "columns": columns,
        "truncated": truncated
    })
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

#[cfg(test)]
#[path = "presentation/tests.rs"]
mod tests;
