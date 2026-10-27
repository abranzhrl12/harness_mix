use super::*;
use crate::contracts::Request;
use serde_json::json;

fn parse(value: serde_json::Value) -> Result<SqlArgs, serde_json::Error> {
    serde_json::from_value(value)
}

#[test]
fn query_defaults_are_explicit_and_preserve_engine_selection() {
    for value in [
        json!({"sql":"SELECT 1"}),
        json!({"action":"query","sql":"SELECT 1"}),
    ] {
        let SqlArgs::Query(args) = parse(value).unwrap() else {
            panic!("expected query")
        };
        assert_eq!(args.engine, Some(SqlEngine::SqlServer));
    }
    let SqlArgs::Query(args) = parse(json!({"sql":"SELECT 1","engine":"postgresql"})).unwrap()
    else {
        panic!("expected query")
    };
    assert_eq!(args.engine, Some(SqlEngine::Postgresql));
    assert!(parse(json!({"query":"SELECT 1"})).is_err());
}

#[test]
fn map_action_parses_optional_schema_and_limit() {
    let SqlArgs::Map(args) = parse(json!({"action":"map"})).unwrap() else {
        panic!("expected map")
    };
    assert_eq!(args.engine, None);
    assert_eq!(args.schema, None);
    assert_eq!(args.limit, None);

    let SqlArgs::Map(args) =
        parse(json!({"action":"map","engine":"postgresql","schema":"public","limit":25})).unwrap()
    else {
        panic!("expected map")
    };
    assert_eq!(args.engine, Some(SqlEngine::Postgresql));
    assert_eq!(args.schema.as_deref(), Some("public"));
    assert_eq!(args.limit, Some(25));

    // Unknown fields never reach the engine.
    assert!(parse(json!({"action":"map","sql":"SELECT 1"})).is_err());
    assert!(matches!(
        parse(json!({"action":"map"})).unwrap().into_request(),
        Request::SqlMap(_)
    ));
}

#[test]
fn invalid_requests_show_complete_shape_without_echoing_input() {
    let error = parse(json!({"action":"query","sql":42,"engine":"password=secret"}))
        .unwrap_err()
        .to_string();
    for field in [
        "sql",
        "action",
        "engine",
        "parameters",
        "session",
        "Example",
    ] {
        assert!(error.contains(field));
    }
    assert!(!error.contains("password=secret"));
    let error = parse(json!({"action":"unknown-secret","sql":"SELECT private"}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("status"));
    assert!(error.contains("search"));
    assert!(error.contains("describe"));
    assert!(!error.contains("unknown-secret"));
    assert!(!error.contains("SELECT private"));
}

#[test]
fn describe_accepts_object_type_alias_and_object_names() {
    for (value, routine) in [
        ("procedure", true),
        ("function", true),
        ("table", false),
        ("view", false),
        ("routine", true),
        ("relation", false),
    ] {
        for field in ["kind", "objectType"] {
            let mut input = json!({"action":"describe","name":"dbo.Object"});
            input[field] = json!(value);
            let SqlArgs::Describe(args) = parse(input).unwrap() else {
                panic!("expected describe")
            };
            assert_eq!(matches!(args.kind, Some(SqlDescribeKind::Routine)), routine);
        }
    }
    assert!(parse(
        json!({"action":"describe","name":"dbo.Object","kind":"routine","objectType":"procedure"})
    )
    .is_err());
}

#[test]
fn describe_qualified_table_needs_no_explicit_type() {
    for name in ["taller.ORDEN_SERVICIO", "dbo.Customer"] {
        let SqlArgs::Describe(args) = parse(json!({"action":"describe","name":name})).unwrap()
        else {
            panic!("expected describe")
        };
        assert_eq!(args.name, name);
        assert!(args.kind.is_none());
    }
}

#[cfg(feature = "schema")]
#[test]
fn describe_schema_requires_neither_kind_nor_object_type() {
    let schema = serde_json::to_value(schemars::schema_for!(SqlDescribeArgs)).unwrap();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("name")));
    assert!(!required.contains(&json!("kind")));
    assert!(!required.contains(&json!("objectType")));
    assert!(schema["properties"].get("objectType").is_some());
}

#[cfg(feature = "schema")]
#[test]
fn schema_query_accepts_implicit_action() {
    let schema = serde_json::to_value(schemars::schema_for!(SqlArgs)).unwrap();
    let branches = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .unwrap()
        .as_array()
        .unwrap();
    let query = branches
        .iter()
        .find(|branch| branch["properties"].get("sql").is_some())
        .unwrap();
    let required = query["required"].as_array().unwrap();
    assert!(required.contains(&json!("sql")));
    assert!(!required.contains(&json!("action")));
}

#[test]
fn each_action_maps_to_its_existing_request() {
    let cases = [
        (json!({"action":"status"}), "status"),
        (json!({"action":"status","connect":true}), "status"),
        (
            json!({"action":"search","query":"cliente","limit":5}),
            "search",
        ),
        (json!({"action":"map","schema":"public","limit":10}), "map"),
        (
            json!({"action":"describe","name":"dbo.Cliente"}),
            "describe",
        ),
        (json!({"action":"query","sql":"SELECT 1"}), "query"),
    ];
    for (value, action) in cases {
        let request = parse(value).unwrap().into_request();
        let matches = match (&request, action) {
            (Request::SqlStatus(_), "status") => true,
            (Request::SqlSearch(_), "search") => true,
            (Request::SqlMap(_), "map") => true,
            (Request::SqlDescribe(_), "describe") => true,
            (Request::SqlQuery(_), "query") => true,
            _ => false,
        };
        assert!(matches, "action {action}");
        assert_eq!(request.name(), "harness_sql");
    }
}

#[test]
fn rejects_missing_required_fields_per_action() {
    assert!(parse(json!({"action":"query"})).is_err());
    assert!(parse(json!({"action":"search"})).is_err());
    assert!(parse(json!({"action":"describe"})).is_err());
    assert!(parse(json!({})).is_err());
    assert!(parse(json!({"action":"drop"})).is_err());
}

#[test]
fn rejects_fields_that_belong_to_another_action() {
    assert!(parse(json!({"action":"status","sql":"SELECT 1"})).is_err());
    assert!(parse(json!({"action":"query","sql":"SELECT 1","query":"x"})).is_err());
    assert!(parse(json!({"action":"search","query":"x","unknown":1})).is_err());
}

#[test]
fn accepts_session_in_every_action() {
    for value in [
        json!({"action":"status","session":"s"}),
        json!({"action":"search","query":"x","session":"s"}),
        json!({"action":"map","session":"s"}),
        json!({"action":"describe","name":"x","session":"s"}),
        json!({"action":"query","sql":"SELECT 1","session":"s"}),
    ] {
        assert!(parse(value).is_ok());
    }
}

#[cfg(feature = "schema")]
#[test]
fn schema_is_an_object_with_one_branch_per_action() {
    let schema = serde_json::to_value(schemars::schema_for!(SqlArgs)).unwrap();
    assert_eq!(
        schema["type"], "object",
        "MCP requires a root object schema"
    );
    let text = schema.to_string();
    for action in ["status", "search", "map", "describe", "query"] {
        assert!(
            text.contains(&format!("\"{action}\"")),
            "schema lacks {action}"
        );
    }
    assert!(schema.get("oneOf").is_some() || schema.get("anyOf").is_some());
}

#[test]
fn query_accepts_the_query_alias_under_explicit_action() {
    let SqlArgs::Query(args) = parse(json!({"action":"query","query":"SELECT 1"})).unwrap()
    else {
        panic!("expected query")
    };
    assert_eq!(args.sql, "SELECT 1");
    // Action-less calls keep requiring the canonical sql field.
    assert!(parse(json!({"query":"SELECT 1"})).is_err());
}

#[test]
fn describe_folds_optional_schema_into_qualified_name() {
    let SqlArgs::Describe(args) =
        parse(json!({"action":"describe","name":"Customer","schema":"dbo"})).unwrap()
    else {
        panic!("expected describe")
    };
    assert_eq!(args.schema.as_deref(), Some("dbo"));

    let SqlArgs::Describe(_) = parse(json!({
        "action":"describe","name":"dbo.Customer","schema":"dbo"
    })).unwrap() else {
        panic!("expected describe")
    };
    // Unknown fields are still rejected.
    assert!(parse(json!({"action":"describe","name":"dbo.Customer","sql":"SELECT 1"})).is_err());
}

#[test]
fn sql_stays_read_only_with_no_ddl_channel() {
    // Read-only policy: SELECT/WITH only. CREATE/ALTER/DROP and other write
    // paths are rejected by the execution preflight; applying schema changes
    // happens outside the harness (SSMS or release), never from a tool call.
    assert!(assert_sql_read_only("SELECT 1").is_ok());
    assert!(assert_sql_read_only("CREATE PROCEDURE dbo.x AS BEGIN END").is_err());
    assert!(assert_sql_read_only("ALTER TABLE dbo.x DROP COLUMN c").is_err());
    assert!(assert_sql_read_only("DROP PROCEDURE dbo.x").is_err());
}
