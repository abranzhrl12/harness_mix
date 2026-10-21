#[cfg(feature = "schema")]
use super::compat::batch_job_alias_schema;
use super::{ExploreArgs, GitArgs, Request, SymbolsArgs};
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
    #[serde(rename = "harness_explore")]
    Explore,
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
#[derive(Debug)]
pub struct BatchDecodeError {
    pub field: Option<String>,
    pub reason: String,
}

impl BatchTool {
    pub fn name(self) -> &'static str {
        match self {
            Self::Config => "harness_config",
            Self::Read => "harness_read",
            Self::Search => "harness_search",
            Self::Symbols => "harness_symbols",
            Self::Detail => "harness_detail",
            Self::Git => "harness_git",
            Self::Sql => "harness_sql",
            Self::Xlsx => "harness_xlsx",
            Self::Svg => "harness_svg",
            Self::Explore => "harness_explore",
        }
    }
}

impl BatchJob {
    pub fn into_request(self, session: Option<&str>) -> Result<Request, crate::Failure> {
        self.into_request_detailed(session)
            .map_err(|_| ("ARGUMENT", "Invalid batch child arguments; no jobs started"))
    }

    pub fn into_request_detailed(
        mut self,
        session: Option<&str>,
    ) -> Result<Request, BatchDecodeError> {
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
            BatchTool::Explore => {
                serde_json::from_value::<ExploreArgs>(value).map(Request::Explore)
            }
        };
        decoded.map_err(|error| {
            let message = error.to_string();
            if let Some(field) = message.strip_prefix("missing field `") {
                return BatchDecodeError {
                    field: field.strip_suffix('`').map(str::to_owned),
                    reason: "required field is missing".to_string(),
                };
            }
            if let Some(field) = message.strip_prefix("unknown field `") {
                let name = field
                    .split_once('`')
                    .map(|(name, _)| name)
                    .unwrap_or(field)
                    .chars()
                    .take(80)
                    .collect::<String>();
                return BatchDecodeError {
                    field: Some(name),
                    reason: "unknown field; check its spelling".to_string(),
                };
            }
            BatchDecodeError {
                field: None,
                // Serde errors can interpolate submitted values (even text resembling
                // an "expected" suffix). Never copy their free-form contents here.
                reason: "arguments do not match the selected tool schema".to_string(),
            }
        })
    }
}
