//! Language-neutral declaration model shared by parser adapters.
use serde::Serialize;

#[derive(Serialize)]
pub struct Symbol {
    pub name: String,
    pub container: String,
    pub kind: &'static str,
    pub start: usize,
    pub end: usize,
    pub signature: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub doc: String,
    #[serde(rename = "symbolId")]
    pub id: String,
    #[serde(skip)]
    pub bytes: std::ops::Range<usize>,
}
