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

/// Unified XLSX tool contract.
/// - If `sheet` is omitted, it performs an inspect (metadata, sheet names, dimensions).
/// - If `sheet` is provided, it reads the requested sheet/range.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(deny_unknown_fields)]
pub struct XlsxArgs {
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    pub path: String,
    pub sheet: Option<String>,
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

impl XlsxArgs {
    pub fn into_request(self) -> crate::contracts::Request {
        if let Some(sheet) = self.sheet {
            crate::contracts::Request::XlsxRead(XlsxReadArgs {
                external_scope: self.external_scope,
                path: self.path,
                sheet,
                range: self.range,
                row_offset: self.row_offset,
                page_size: self.page_size,
                expected_hash: self.expected_hash,
                formulas: self.formulas,
                session: self.session,
            })
        } else {
            crate::contracts::Request::XlsxInspect(XlsxInspectArgs {
                external_scope: self.external_scope,
                path: self.path,
                expected_hash: self.expected_hash,
                session: self.session,
            })
        }
    }
}
