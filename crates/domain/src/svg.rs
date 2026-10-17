//! SVG contracts and data structures.
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SvgMode {
    #[default]
    Inspect,
    Preview,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SvgArgs {
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    pub path: String,
    #[serde(default)]
    pub mode: Option<SvgMode>,
    #[serde(rename = "maxSize")]
    pub max_size: Option<u32>,
    pub session: Option<String>,
}
