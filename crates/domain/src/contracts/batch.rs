#[cfg(feature = "schema")]
use super::compat::batch_job_alias_schema;
use super::{GitArgs, Request, SymbolsArgs};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum BatchTool {
    #[serde(rename = "harness_config")]
    Config,
    #[serde(rename = "harness_read", alias = "read")]
    Read,
    #[serde(rename = "harness_search", alias = "search")]
    Search,
    #[serde(rename = "harness_symbols")]
    Symbols,
    #[serde(rename = "harness_detail")]
    Detail,
    #[serde(rename = "harness_git")]
    Git,
    #[serde(rename = "harness_sql")]
    Sql,
    #[serde(rename = "harness_xlsx")]
    Xlsx,
    #[serde(rename = "harness_svg")]
    Svg,
}
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(transform = batch_job_alias_schema))]
pub struct BatchJob {
    pub tool: BatchTool,
    #[serde(alias = "args")]
    pub arguments: serde_json::Map<String, Value>,
}
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct BatchArgs {
    pub jobs: Vec<BatchJob>,
    /// Concurrent independent reads, from 2 to 4 (default 2).
    #[serde(rename = "maxConcurrent")]
    #[cfg_attr(feature = "schema", schemars(range(min = 2, max = 4)))]
    pub max_concurrent: Option<usize>,
    pub session: Option<String>,
}
impl BatchJob {
    pub fn into_request(mut self, session: Option<&str>) -> Result<Request, crate::Failure> {
        if let Some(session) = session {
            self.arguments
                .insert("session".into(), Value::String(session.into()));
        }
        let value = Value::Object(self.arguments);
        let decoded = match self.tool {
            BatchTool::Config => serde_json::from_value(value).map(Request::Config),
            BatchTool::Read => serde_json::from_value(value).map(Request::Read),
            BatchTool::Search => serde_json::from_value(value).map(Request::Search),
            BatchTool::Symbols => {
                serde_json::from_value::<SymbolsArgs>(value).map(SymbolsArgs::into_request)
            }
            BatchTool::Detail => serde_json::from_value(value).map(Request::Detail),
            BatchTool::Git => serde_json::from_value::<GitArgs>(value).map(GitArgs::into_request),
            BatchTool::Sql => serde_json::from_value::<crate::sql::SqlArgs>(value)
                .map(crate::sql::SqlArgs::into_request),
            BatchTool::Xlsx => serde_json::from_value::<crate::xlsx::XlsxArgs>(value)
                .map(crate::xlsx::XlsxArgs::into_request),
            BatchTool::Svg => serde_json::from_value(value).map(Request::Svg),
        };
        decoded.map_err(|_| ("ARGUMENT", "Invalid batch child arguments; no jobs started"))
    }
}
