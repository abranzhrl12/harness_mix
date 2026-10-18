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
    pub schema: Option<String>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SqlDescribeKind {
    Relation,
    Routine,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SqlDescribeArgs {
    pub engine: Option<SqlEngine>,
    pub name: String,
    pub kind: Option<SqlDescribeKind>,
    pub sections: Option<Vec<String>>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SqlQueryArgs {
    pub engine: Option<SqlEngine>,
    pub sql: String,
    pub parameters: Option<serde_json::Map<String, serde_json::Value>>,
    pub session: Option<String>,
}

/// Unified SQL Server tool. `action` selects the operation and its required fields:
/// status {connect?}, search {query, objectType?, schema?, limit?},
/// describe {name, kind?, sections?}, query {sql, parameters?}.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SqlArgs {
    Status(SqlStatusArgs),
    Search(SqlSearchArgs),
    Describe(SqlDescribeArgs),
    Query(SqlQueryArgs),
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
