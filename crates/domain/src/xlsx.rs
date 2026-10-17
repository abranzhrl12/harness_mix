//! XLSX contracts and data structures.
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct XlsxInspectArgs {
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    pub path: String,
    #[serde(rename = "expectedHash")]
    pub expected_hash: Option<String>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct XlsxReadArgs {
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    pub path: String,
    pub sheet: String,
    pub range: Option<String>,
    #[serde(rename = "rowOffset")]
    pub row_offset: Option<usize>,
    #[serde(rename = "pageSize")]
    pub page_size: Option<usize>,
    #[serde(rename = "expectedHash")]
    pub expected_hash: Option<String>,
    pub formulas: Option<bool>,
    pub session: Option<String>,
}
