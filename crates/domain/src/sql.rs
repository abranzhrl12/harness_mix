//! SQL Server contracts and strict read-only query policy.
use serde::{Deserialize, Serialize};

mod readonly;
mod schema_compat;

pub use readonly::assert_sql_read_only;

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
    /// OPTIONAL: Database engine (`sqlserver` or `postgresql`).
    pub engine: Option<SqlEngine>,
    /// OPTIONAL: Attempt a live connection handshake check.
    pub connect: Option<bool>,
    /// OPTIONAL: Session token from `harness_bootstrap`.
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SqlObjectType {
    Table,
    View,
    Column,
    #[serde(
        alias = "sp",
        alias = "stored_procedure",
        alias = "storedProcedure",
        alias = "proc",
        alias = "routine"
    )]
    Procedure,
    #[serde(alias = "fn", alias = "func")]
    Function,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SqlSearchArgs {
    /// OPTIONAL: Database engine (`sqlserver` or `postgresql`).
    pub engine: Option<SqlEngine>,
    /// REQUIRED for `action=search`: Search term (substring of table, view, column, procedure, or function name).
    #[serde(alias = "name", alias = "pattern")]
    pub query: String,
    /// OPTIONAL: Maximum number of matching objects to return.
    pub limit: Option<usize>,
    /// OPTIONAL: Filter by object type (`table`, `view`, `column`, `procedure`, `function`).
    #[serde(rename = "objectType", alias = "kind")]
    pub object_type: Option<SqlObjectType>,
    /// OPTIONAL: Schema filter; omitted, null or `*` searches every schema allowed by the connection policy.
    pub schema: Option<String>,
    /// OPTIONAL: Session token from `harness_bootstrap`.
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", schemars(transform = crate::sql::schema_compat::describe_kind_aliases))]
pub enum SqlDescribeKind {
    #[serde(alias = "table", alias = "view")]
    Relation,
    #[serde(
        alias = "procedure",
        alias = "function",
        alias = "sp",
        alias = "stored_procedure",
        alias = "storedProcedure",
        alias = "proc",
        alias = "fn",
        alias = "func"
    )]
    Routine,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(transform = crate::sql::schema_compat::describe_field_alias))]
pub struct SqlDescribeArgs {
    /// OPTIONAL: Database engine (`sqlserver` or `postgresql`).
    pub engine: Option<SqlEngine>,
    /// REQUIRED for `action=describe`: Target object name (`dbo.MyTable` or `dbo.MyProcedure`). Aliases accepted: `object`, `objectName`, `table`, `procedure`, `routine`.
    #[serde(
        alias = "object",
        alias = "objectName",
        alias = "object_name",
        alias = "table",
        alias = "procedure",
        alias = "routine"
    )]
    pub name: String,
    /// OPTIONAL: Schema prefix when `name` is unqualified; the qualified
    /// resolver for each engine handles folding both into one name.
    pub schema: Option<String>,
    /// OPTIONAL: Disambiguate object category (`relation`/`table`/`view` vs `routine`/`procedure`/`function`). OMIT by default — auto-detected from catalog. Alias: `objectType`.
    #[serde(alias = "objectType")]
    pub kind: Option<SqlDescribeKind>,
    /// OPTIONAL: Specific metadata sections to include (e.g. `["columns", "parameters", "definition"]`).
    pub sections: Option<Vec<String>>,
    /// OPTIONAL: PostgreSQL only — bypass and replace an opt-in schema-cache entry; never use stale fallback.
    pub refresh: Option<bool>,
    /// OPTIONAL: Session token from `harness_bootstrap`.
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SqlMapArgs {
    /// OPTIONAL: Database engine (`sqlserver` or `postgresql`).
    pub engine: Option<SqlEngine>,
    /// OPTIONAL: Schema to map; omitted, null or `*` maps every schema allowed by the policy.
    pub schema: Option<String>,
    /// OPTIONAL: Maximum number of relations to report.
    pub limit: Option<usize>,
    /// OPTIONAL: Session token from `harness_bootstrap`.
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SqlQueryArgs {
    /// OPTIONAL: Database engine (`sqlserver` or `postgresql`, defaults to `sqlserver`).
    #[serde(default = "default_query_engine")]
    pub engine: Option<SqlEngine>,
    /// REQUIRED for `action=query`: Read-only SQL text (`SELECT` / `WITH` only). `query` is accepted as an alias under `action=query`.
    #[serde(alias = "query")]
    pub sql: String,
    /// OPTIONAL: Named query parameters map.
    pub parameters: Option<serde_json::Map<String, serde_json::Value>>,
    /// OPTIONAL: Session token from `harness_bootstrap`.
    pub session: Option<String>,
}

/// Unified SQL Server / PostgreSQL tool. `action` selects the operation and its required fields:
/// - `describe`: REQUIRED `{action: "describe", name: string}`. OPTIONAL `{schema?, kind?/objectType? (auto-detected if omitted), sections?, engine?, session?}`.
/// - `search`: REQUIRED `{action: "search", query: string}`. OPTIONAL `{objectType?, schema?, limit?, engine?, session?}`.
/// - `query`: REQUIRED `{sql: string}` (`action: "query"` optional when `sql` is provided). OPTIONAL `{parameters?, engine?, session?}`.
/// - `map`: REQUIRED `{action: "map"}`. OPTIONAL `{schema?, limit?, engine?, session?}`.
/// - `status`: REQUIRED `{action: "status"}`. OPTIONAL `{connect?, engine?, session?}`.
#[derive(Debug, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[cfg_attr(feature = "schema", schemars(transform = crate::sql::schema_compat::optional_query_action))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SqlArgs {
    Status(SqlStatusArgs),
    Search(SqlSearchArgs),
    Map(SqlMapArgs),
    Describe(SqlDescribeArgs),
    Query(SqlQueryArgs),
}

fn default_query_engine() -> Option<SqlEngine> {
    Some(SqlEngine::SqlServer)
}

/// Safely extracts structural field diagnostics (`missing field`, `unknown field`, `duplicate field`)
/// from a `serde_json::Error` without echoing any user-submitted values, SQL, or credentials.
pub fn extract_safe_serde_field_diagnostic(error_text: &str) -> Option<String> {
    if let Some(rest) = error_text.strip_prefix("missing field `") {
        if let Some((field, _)) = rest.split_once('`') {
            return Some(format!("Missing required field `{field}`."));
        }
    }
    if let Some(rest) = error_text.strip_prefix("unknown field `") {
        if let Some((field, tail)) = rest.split_once('`') {
            let safe_field: String = field
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .take(48)
                .collect();
            let expected_hint = tail
                .split_once("expected ")
                .map(|(_, exp)| format!(" (expected {exp})"))
                .unwrap_or_default();
            return Some(format!(
                "Unknown field `{safe_field}`{expected_hint}."
            ));
        }
    }
    if let Some(rest) = error_text.strip_prefix("duplicate field `") {
        if let Some((field, _)) = rest.split_once('`') {
            return Some(format!(
                "Duplicate/conflicting field `{field}` (do not pass both canonical name and alias)."
            ));
        }
    }
    None
}

fn format_sql_action_error(action_spec: &str, err: &serde_json::Error) -> String {
    match extract_safe_serde_field_diagnostic(&err.to_string()) {
        Some(prefix) => format!("{prefix} {action_spec}"),
        None => action_spec.to_owned(),
    }
}

impl<'de> Deserialize<'de> for SqlArgs {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        const OVERVIEW: &str = "harness_sql expects an object: query {sql: string, engine?: sqlserver|postgresql, parameters?: object, session?: string}; action defaults to query only when sql is present. Other actions: status {action: status, engine?, connect?, session?}; search {action: search, query: string, engine?, objectType?, schema?, limit?, session?}; map {action: map, engine?, schema?, limit?, session?}; describe {action: describe, name: string, engine?, schema?, kind?: relation|routine (optional, auto-detected if omitted; objectType alias accepted), sections?, refresh?, session?}. Example: {\"sql\":\"SELECT 1\"}";
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
        // Never include serde's raw value payloads: extract only safe structural field diagnostics.
        match action.as_str() {
            "query" => serde_json::from_value(value).map(Self::Query).map_err(|err| {
                D::Error::custom(format_sql_action_error(
                    "Invalid harness_sql query. REQUIRED: {sql: string} (query accepted as alias when action=\"query\"). OPTIONAL: {action: \"query\", engine: \"sqlserver\"|\"postgresql\", parameters: object, session: string}. Read-only policy: SELECT/WITH only; CREATE and ALTER scripts are not executed here — apply them through SSMS. Example: {\"action\":\"query\",\"sql\":\"SELECT 1\"}",
                    &err,
                ))
            }),
            "status" => serde_json::from_value(value).map(Self::Status).map_err(|err| {
                D::Error::custom(format_sql_action_error(
                    "Invalid harness_sql status. REQUIRED: {action: \"status\"}. OPTIONAL: {engine: \"sqlserver\"|\"postgresql\", connect: boolean, session: string}. Example: {\"action\":\"status\"}",
                    &err,
                ))
            }),
            "search" => serde_json::from_value(value).map(Self::Search).map_err(|err| {
                D::Error::custom(format_sql_action_error(
                    "Invalid harness_sql search. REQUIRED: {action: \"search\", query: string}. OPTIONAL: {engine: \"sqlserver\"|\"postgresql\", objectType: \"table\"|\"view\"|\"column\"|\"procedure\"|\"function\", schema: string, limit: integer, session: string}. Example: {\"action\":\"search\",\"query\":\"Customer\"}",
                    &err,
                ))
            }),
            "map" => serde_json::from_value(value).map(Self::Map).map_err(|err| {
                D::Error::custom(format_sql_action_error(
                    "Invalid harness_sql map. REQUIRED: {action: \"map\"}. OPTIONAL: {engine: \"sqlserver\"|\"postgresql\", schema: string, limit: integer, session: string}. Example: {\"action\":\"map\",\"schema\":\"dbo\"}",
                    &err,
                ))
            }),
            "describe" => serde_json::from_value(value).map(Self::Describe).map_err(|err| {
                D::Error::custom(format_sql_action_error(
                    "Invalid harness_sql describe. REQUIRED: {action: \"describe\", name: string} (aliases accepted for name: object, objectName, table, procedure, routine). OPTIONAL: {schema: string, kind: \"relation\"|\"routine\" (or objectType: \"table\"|\"view\"|\"procedure\"|\"function\" — auto-detected if omitted; do not pass both kind and objectType), sections: string[], engine: \"sqlserver\"|\"postgresql\", refresh: boolean, session: string}. Minimal example: {\"action\":\"describe\",\"name\":\"dbo.Customer\"}",
                    &err,
                ))
            }),
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
            Self::Map(args) => Request::SqlMap(args),
            Self::Describe(args) => Request::SqlDescribe(args),
            Self::Query(args) => Request::SqlQuery(args),
        }
    }
}

#[cfg(test)]
#[path = "sql/tests.rs"]
mod unified_tool_tests;
