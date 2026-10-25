//! Ownership manifest format used to protect local skill edits.

use std::collections::BTreeMap;

pub(crate) const PATH: &str = ".agents/skills/.harness-manifest.json";
pub(crate) const FORMAT: u32 = 1;

pub(crate) fn contract_version() -> &'static str {
    harness_domain::tool_contract::manifest()["contractVersion"]
        .as_str()
        .unwrap_or("unknown")
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub(crate) format_version: u32,
    pub(crate) package_version: String,
    pub(crate) contract_version: String,
    pub(crate) files: BTreeMap<String, String>,
}
