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
#[cfg_attr(feature = "schema", schemars(transform = crate::sql::schema_compat::describe_kind_aliases))]
pub enum SqlDescribeKind {
    #[serde(alias = "table", alias = "view")]
    Relation,
    #[serde(alias = "procedure", alias = "function")]
    Routine,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(transform = crate::sql::schema_compat::describe_field_alias))]
pub struct SqlDescribeArgs {
    pub engine: Option<SqlEngine>,
    pub name: String,
    /// Optional schema prefix when `name` is unqualified; the qualified
    /// resolver for each engine handles folding both into one name.
    pub schema: Option<String>,
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
pub struct SqlMapArgs {
    pub engine: Option<SqlEngine>,
    /// Schema to map; omitted, null or * maps every schema allowed by the policy.
    pub schema: Option<String>,
    /// Maximum number of relations to report.
    pub limit: Option<usize>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SqlQueryArgs {
    #[serde(default = "default_query_engine")]
    pub engine: Option<SqlEngine>,
    /// SQL text to run; `query` is accepted as an alias under action=query.
    #[serde(alias = "query")]
    pub sql: String,
    pub parameters: Option<serde_json::Map<String, serde_json::Value>>,
    pub session: Option<String>,
}

/// Unified SQL Server tool. `action` selects the operation and its required fields:
/// status {connect?}, search {query, objectType?, schema?, limit?},
/// map {schema?, limit?}, describe {name, kind?, sections?}, query {sql, parameters?}.
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

impl<'de> Deserialize<'de> for SqlArgs {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        const OVERVIEW: &str = "harness_sql expects an object: query {sql: string, engine?: sqlserver|postgresql, parameters?: object, session?: string}; action defaults to query only when sql is present. Other actions: status {action: status, engine?, connect?, session?}; search {action: search, query: string, engine?, objectType?, schema?, limit?, session?}; map {action: map, engine?, schema?, limit?, session?}; describe {action: describe, name: string, engine?, kind?: relation|routine (objectType alias; table/view/procedure/function accepted), sections?, refresh?, session?}. Example: {\"sql\":\"SELECT 1\"}";
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
            "query" => serde_json::from_value(value).map(Self::Query).map_err(|_| D::Error::custom("Invalid harness_sql query. Required: sql: string (query accepted as an alias under action=query). Optional: action=query, engine=sqlserver|postgresql (default sqlserver), parameters: object, session: string. Read-only policy: SELECT/WITH only; CREATE and ALTER scripts are not executed here — apply them through SSMS. Example: {\"action\":\"query\",\"query\":\"SELECT 1\"}")),
            "status" => serde_json::from_value(value).map(Self::Status).map_err(|_| D::Error::custom("Invalid harness_sql status. Required: action=status. Optional: engine=sqlserver|postgresql, connect: boolean, session: string. Example: {\"action\":\"status\"}")),
            "search" => serde_json::from_value(value).map(Self::Search).map_err(|_| D::Error::custom("Invalid harness_sql search. Required: action=search, query: string. Optional: engine=sqlserver|postgresql, objectType=table|view|column|procedure|function, schema: string, limit: integer, session: string. Example: {\"action\":\"search\",\"query\":\"Customer\"}")),
            "map" => serde_json::from_value(value).map(Self::Map).map_err(|_| D::Error::custom("Invalid harness_sql map. Required: action=map. Optional: engine=sqlserver|postgresql, schema: string, limit: integer, session: string. Example: {\"action\":\"map\",\"schema\":\"public\"}")),
            "describe" => serde_json::from_value(value).map(Self::Describe).map_err(|_| D::Error::custom("Invalid harness_sql describe. Required: action=describe, name: string. Optional: engine=sqlserver|postgresql, schema: string (prefixes an unqualified name), kind=relation|routine (objectType alias; table/view/procedure/function accepted), sections: string[], refresh: boolean, session: string. Do not supply both kind and objectType. Example: {\"action\":\"describe\",\"name\":\"Customer\",\"schema\":\"dbo\",\"objectType\":\"table\"}")),
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
