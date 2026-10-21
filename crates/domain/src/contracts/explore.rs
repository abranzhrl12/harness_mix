use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataQueryArgs {
    pub path: Option<String>,
    pub data: Option<Value>,
    /// Stored JSON root from a previous detail, subject to the same TTL.
    #[serde(rename = "resultRef")]
    pub result_ref: Option<String>,
    pub query: String,
    #[serde(rename = "maxChars")]
    pub max_chars: Option<usize>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExploreArgs {
    /// Optional with symbol/symbolId, or filePath alone for non-impact Razor render relationships.
    #[serde(default)]
    pub query: String,
    /// Summary returns signatures and graph counts; detail retains the graph.
    pub view: Option<ExploreView>,
    #[serde(rename = "filePath")]
    pub file_path: Option<String>,
    pub container: Option<String>,
    /// CodeGraph node id from exploration candidates, not a local AST symbol id.
    #[serde(rename = "symbolId")]
    pub symbol_id: Option<String>,
    /// Explicit authorization to write the existing native index before querying.
    /// Omitted/false preserves read-only exploration; no index is created implicitly.
    #[serde(rename = "syncIndex")]
    pub sync_index: Option<bool>,
    pub symbol: Option<String>,
    #[serde(rename = "maxFiles")]
    pub max_files: Option<usize>,
    #[serde(rename = "maxChars")]
    pub max_chars: Option<usize>,
    /// Indexed traversal only: defaults to both directions and one hop.
    pub direction: Option<ExploreDirection>,
    #[serde(rename = "maxDepth")]
    pub max_depth: Option<usize>,
    /// view=impact only: maximum displayed callers, 1..100; defaults to 20.
    pub limit: Option<usize>,
    /// Reject results unless all contributing files match indexed hashes.
    /// This does not certify full index coverage or freshness.
    #[serde(rename = "requireMatchingFiles")]
    pub require_matching_files: Option<bool>,
    #[serde(alias = "sessionId")]
    pub session: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExploreView {
    Summary,
    Full,
    Impact,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExploreDirection {
    Inbound,
    Outbound,
    Both,
}
