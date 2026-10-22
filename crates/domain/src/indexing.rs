//! Portable source observations and indexing reports. No storage or parser types.
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub enum Observation {
    /// Digest of a stable source read, not a size/mtime shortcut.
    Content(String),
    /// Explicit absence verified by the scanner (not permission denied).
    Missing,
    /// Unreadable, unstable, excluded or over budget. Never implies deletion.
    Unchecked,
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub retry_extraction: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
    pub unchecked: Vec<String>,
    pub inventory_complete: bool,
    pub synchronized: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    #[serde(skip)]
    pub observations: BTreeMap<String, Observation>,
    pub discovery_complete: bool,
    pub visited_entries: usize,
    pub reserved_source_bytes: u64,
    pub skipped_entries: usize,
    pub restricted_indexed_files: usize,
    // One policy category per restricted entry; never expose its path or hash.
    pub restricted_indexed_reasons: BTreeMap<&'static str, usize>,
    pub budget_exhausted: bool,
    pub scope_excluded: Vec<String>,
    pub scope: serde_json::Value,
    pub file_diagnostics: Vec<serde_json::Value>,
}
