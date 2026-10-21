use super::{ResolvedSqlObject, RoutineCatalog, routine_response};
use serde_json::{Value, json};

#[test]
fn routine_response_preserves_resolution_and_catalog_budget_metadata() {
    let resolved = ResolvedSqlObject {
        schema: "app".into(),
        name: "refresh]cache".into(),
        object_id: 42,
        kind: "SQL_STORED_PROCEDURE".into(),
    };

    let response = routine_response(
        "refresh]cache",
        &resolved,
        RoutineCatalog {
            definition: Value::Null,
            definition_available: false,
            parameters: vec![json!({"name": "@force", "typeName": "bit"})],
            definition_truncated: false,
            parameters_truncated: true,
        },
    );

    assert_eq!(response["engine"], "sqlserver");
    assert_eq!(response["name"], "refresh]cache");
    assert_eq!(response["resolvedName"], "[app].[refresh]]cache]");
    assert_eq!(response["objectId"], 42);
    assert_eq!(response["kind"], "routine");
    assert_eq!(response["parameters"][0]["name"], "@force");
    assert_eq!(response["parametersComplete"], false);
    assert_eq!(response["truncated"], true);
    assert_eq!(
        response["diagnostics"][0]["code"],
        "SQL_DEFINITION_UNAVAILABLE"
    );
}

#[test]
fn routine_response_does_not_confuse_budget_omission_with_missing_definition() {
    let resolved = ResolvedSqlObject {
        schema: "app".into(),
        name: "refresh_cache".into(),
        object_id: 42,
        kind: "SQL_STORED_PROCEDURE".into(),
    };

    let response = routine_response(
        "refresh_cache",
        &resolved,
        RoutineCatalog {
            definition: Value::Null,
            definition_available: true,
            parameters: Vec::new(),
            definition_truncated: true,
            parameters_truncated: false,
        },
    );

    assert_eq!(response["truncated"], true);
    assert!(response.get("diagnostics").is_none());
}
