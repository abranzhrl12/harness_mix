//! SQL Server contracts and strict read-only query policy.
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum SqlEngine {
    SqlServer,
    Postgresql,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SqlStatusArgs {
    pub engine: Option<SqlEngine>,
    pub connect: Option<bool>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SqlObjectType {
    Table,
    View,
    Column,
    Procedure,
    Function,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SqlSearchArgs {
    pub engine: Option<SqlEngine>,
    pub query: String,
    pub limit: Option<usize>,
    #[serde(rename = "objectType")]
    pub object_type: Option<SqlObjectType>,
    /// Omitted, null or * searches every schema allowed by the connection policy.
    pub schema: Option<String>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", schemars(transform = describe_kind_aliases))]
pub enum SqlDescribeKind {
    #[serde(alias = "table", alias = "view")]
    Relation,
    #[serde(alias = "procedure", alias = "function")]
    Routine,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(transform = describe_field_alias))]
pub struct SqlDescribeArgs {
    pub engine: Option<SqlEngine>,
    pub name: String,
    #[serde(alias = "objectType")]
    pub kind: Option<SqlDescribeKind>,
    pub sections: Option<Vec<String>>,
    /// PostgreSQL: bypass and replace an opt-in schema-cache entry; never use stale fallback.
    pub refresh: Option<bool>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SqlQueryArgs {
    #[serde(default = "default_query_engine")]
    pub engine: Option<SqlEngine>,
    pub sql: String,
    pub parameters: Option<serde_json::Map<String, serde_json::Value>>,
    pub session: Option<String>,
}

/// Unified SQL Server tool. `action` selects the operation and its required fields:
/// status {connect?}, search {query, objectType?, schema?, limit?},
/// describe {name, kind?, sections?}, query {sql, parameters?}.
#[derive(Debug, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[cfg_attr(feature = "schema", schemars(transform = optional_query_action))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SqlArgs {
    Status(SqlStatusArgs),
    Search(SqlSearchArgs),
    Describe(SqlDescribeArgs),
    Query(SqlQueryArgs),
}

fn default_query_engine() -> Option<SqlEngine> {
    Some(SqlEngine::SqlServer)
}

#[cfg(feature = "schema")]
fn describe_kind_aliases(schema: &mut schemars::Schema) {
    schema.insert(
        "enum".into(),
        serde_json::json!([
            "relation",
            "routine",
            "table",
            "view",
            "procedure",
            "function"
        ]),
    );
}

#[cfg(feature = "schema")]
fn describe_field_alias(schema: &mut schemars::Schema) {
    if let Some(properties) = schema
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
    {
        if let Some(kind) = properties.get("kind").cloned() {
            properties.insert("objectType".into(), kind);
        }
    }
    // As with serde alias decoding, callers must choose one spelling.
    schema.insert(
        "not".into(),
        serde_json::json!({"required":["kind", "objectType"]}),
    );
}

/// Keep the advertised query branch consistent with implicit action=query.
#[cfg(feature = "schema")]
fn optional_query_action(schema: &mut schemars::Schema) {
    for key in ["oneOf", "anyOf"] {
        if let Some(branches) = schema
            .get_mut(key)
            .and_then(serde_json::Value::as_array_mut)
        {
            for branch in branches {
                if branch["properties"].get("sql").is_some() {
                    if let Some(required) = branch["required"].as_array_mut() {
                        required.retain(|field| field != "action");
                    }
                }
            }
        }
    }
}

impl<'de> Deserialize<'de> for SqlArgs {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        const OVERVIEW: &str = "harness_sql expects an object: query {sql: string, engine?: sqlserver|postgresql, parameters?: object, session?: string}; action defaults to query only when sql is present. Other actions: status {action: status, engine?, connect?, session?}; search {action: search, query: string, engine?, objectType?, schema?, limit?, session?}; describe {action: describe, name: string, engine?, kind?: relation|routine (objectType alias; table/view/procedure/function accepted), sections?, refresh?, session?}. Example: {\"sql\":\"SELECT 1\"}";
        let value = serde_json::Value::deserialize(deserializer)?;
        let mut object = value
            .as_object()
            .cloned()
            .ok_or_else(|| D::Error::custom(OVERVIEW))?;
        let action = match object.remove("action") {
            Some(serde_json::Value::String(action)) => action,
            None if object.contains_key("sql") => "query".to_owned(),
            _ => return Err(D::Error::custom(OVERVIEW)),
        };
        let value = serde_json::Value::Object(object);
        // Never include serde's raw error: it can echo SQL, credentials or user values.
        match action.as_str() {
            "query" => serde_json::from_value(value).map(Self::Query).map_err(|_| D::Error::custom("Invalid harness_sql query. Required: sql: string. Optional: action=query, engine=sqlserver|postgresql (default sqlserver), parameters: object, session: string. Example: {\"sql\":\"SELECT 1\"}")),
            "status" => serde_json::from_value(value).map(Self::Status).map_err(|_| D::Error::custom("Invalid harness_sql status. Required: action=status. Optional: engine=sqlserver|postgresql, connect: boolean, session: string. Example: {\"action\":\"status\"}")),
            "search" => serde_json::from_value(value).map(Self::Search).map_err(|_| D::Error::custom("Invalid harness_sql search. Required: action=search, query: string. Optional: engine=sqlserver|postgresql, objectType=table|view|column|procedure|function, schema: string, limit: integer, session: string. Example: {\"action\":\"search\",\"query\":\"Customer\"}")),
            "describe" => serde_json::from_value(value).map(Self::Describe).map_err(|_| D::Error::custom("Invalid harness_sql describe. Required: action=describe, name: string. Optional: engine=sqlserver|postgresql, kind=relation|routine (objectType alias; table/view/procedure/function accepted), sections: string[], refresh: boolean, session: string. Do not supply both kind and objectType. Example: {\"action\":\"describe\",\"name\":\"dbo.Customer\",\"objectType\":\"table\"}")),
            _ => Err(D::Error::custom(OVERVIEW)),
        }
    }
}

impl SqlArgs {
    /// Maps the unified action to the existing per-operation request, so the
    /// application layer and adapters keep a single implementation per action.
    pub fn into_request(self) -> crate::contracts::Request {
        use crate::contracts::Request;
        match self {
            Self::Status(args) => Request::SqlStatus(args),
            Self::Search(args) => Request::SqlSearch(args),
            Self::Describe(args) => Request::SqlDescribe(args),
            Self::Query(args) => Request::SqlQuery(args),
        }
    }
}

/// Conservative preflight read-only validator for T-SQL queries.
/// Replicates the strict rules of sql-readonly.cjs.
pub fn assert_sql_read_only(sql: &str) -> Result<(), (&'static str, &'static str)> {
    let trimmed = sql.trim();
    if trimmed.is_empty() || trimmed.len() > 16_000 {
        return Err(("SQL_READONLY", "Consulta vacía o excede 16000 caracteres"));
    }

    const FORBIDDEN: &[&str] = &[
        "UPDATE",
        "DELETE",
        "INSERT",
        "MERGE",
        "DROP",
        "ALTER",
        "CREATE",
        "TRUNCATE",
        "EXEC",
        "EXECUTE",
        "GRANT",
        "DENY",
        "REVOKE",
        "INTO",
        "OPENQUERY",
        "OPENROWSET",
        "OPENDATASOURCE",
        "NEXT",
        "UPDLOCK",
        "XLOCK",
        "TABLOCK",
        "TABLOCKX",
        "HOLDLOCK",
        "SERIALIZABLE",
        "REPEATABLEREAD",
        "BEGIN",
        "COMMIT",
        "ROLLBACK",
        "SAVE",
        "SET",
        "USE",
        "DECLARE",
        "WAITFOR",
        "DBCC",
        "BACKUP",
        "RESTORE",
        "BULK",
        "KILL",
        "SHUTDOWN",
        "RECONFIGURE",
        "GO",
    ];

    let mut tokens: Vec<String> = Vec::new();
    let mut depth: isize = 0;
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        let next = if i + 1 < chars.len() {
            Some(chars[i + 1])
        } else {
            None
        };

        if c.is_whitespace() {
            i += 1;
            continue;
        }

        // Single-line comment: -- ...
        if c == '-' && next == Some('-') {
            i += 2;
            while i < chars.len() && chars[i] != '\r' && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // Multi-line comment: /* ... */
        if c == '/' && next == Some('*') {
            i += 2;
            let mut nesting = 1;
            while i < chars.len() && nesting > 0 {
                if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
                    nesting += 1;
                    if nesting > 100 {
                        return Err(("SQL_READONLY", "Comentario demasiado anidado"));
                    }
                    i += 2;
                } else if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                    nesting -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if nesting > 0 {
                return Err(("SQL_READONLY", "Comentario multilínea sin cerrar"));
            }
            continue;
        }

        // Quoted string or identifier: '...', "...", [...]
        if c == '\'' || c == '"' || c == '[' {
            let end_char = if c == '[' { ']' } else { c };
            let mut closed = false;
            i += 1;
            while i < chars.len() {
                if chars[i] == end_char {
                    if i + 1 < chars.len() && chars[i + 1] == end_char {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err(("SQL_READONLY", "Identificador o literal sin cerrar"));
            }
            tokens.push("<quoted>".to_string());
            continue;
        }

        // Word tokens
        if c.is_ascii_alphabetic() || c == '_' || c == '@' || c == '#' {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '_'
                    || chars[i] == '@'
                    || chars[i] == '$'
                    || chars[i] == '#')
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let upper = word.to_ascii_uppercase();
            if FORBIDDEN.contains(&upper.as_str()) {
                return Err((
                    "SQL_READONLY",
                    "SQL_READONLY: solo se permite un SELECT sin operaciones de escritura ni bloqueos exclusivos",
                ));
            }
            tokens.push(upper);
            continue;
        }

        if c == '(' {
            depth += 1;
            if depth > 100 {
                return Err(("SQL_READONLY", "Paréntesis demasiado anidados"));
            }
        } else if c == ')' {
            depth -= 1;
            if depth < 0 {
                return Err(("SQL_READONLY", "Paréntesis desbalanceados"));
            }
        }

        tokens.push(c.to_string());
        i += 1;
    }

    if depth != 0 {
        return Err(("SQL_READONLY", "Paréntesis desbalanceados"));
    }

    // Optional leading / trailing semicolon
    if tokens.first().map(|s| s.as_str()) == Some(";") {
        tokens.remove(0);
    }
    if tokens.last().map(|s| s.as_str()) == Some(";") {
        tokens.pop();
    }

    let first = tokens.first().map(|s| s.as_str());
    if first != Some("SELECT") && first != Some("WITH") {
        return Err((
            "SQL_READONLY",
            "La consulta debe comenzar con SELECT o WITH",
        ));
    }

    if tokens.iter().any(|s| s == ";") {
        return Err((
            "SQL_READONLY",
            "Múltiples declaraciones no permitidas en consulta SQL",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod unified_tool_tests {
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
        assert!(parse(json!({"action":"describe","name":"dbo.Object","kind":"routine","objectType":"procedure"})).is_err());
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
        for action in ["status", "search", "describe", "query"] {
            assert!(
                text.contains(&format!("\"{action}\"")),
                "schema lacks {action}"
            );
        }
        assert!(schema.get("oneOf").is_some() || schema.get("anyOf").is_some());
    }
}
