use super::Request;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct OutlineArgs {
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    /// REQUIRED for `action=outline`: File path to outline.
    pub path: String,
    /// OPTIONAL: Substring filter for symbol names.
    pub query: Option<String>,
    #[serde(rename = "maxChars")]
    pub max_chars: Option<usize>,
    #[serde(rename = "filterKind")]
    pub filter_kind: Option<OutlineFilterKind>,
    pub view: Option<OutlineView>,
    pub session: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum OutlineFilterKind {
    Method,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum OutlineView {
    Compact,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SymbolView {
    Full,
    Signature,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ReadSymbolArgs {
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    /// REQUIRED for `action=read`: File path containing the symbol.
    pub path: String,
    /// REQUIRED for `action=read`: Exact symbol name (`symbol`, not `name`).
    pub symbol: String,
    /// OPTIONAL: Enclosing class/type/module container to disambiguate homonyms.
    pub container: Option<String>,
    /// OPTIONAL: Exact symbolId selector from outline.
    #[serde(rename = "symbolId")]
    pub symbol_id: Option<String>,
    /// OPTIONAL: `full` (default, returns exact source) or `signature` (declaration only).
    pub view: Option<SymbolView>,
    /// Copy data.session from bootstrap.
    #[serde(alias = "sessionId")]
    pub session: Option<String>,
}

/// Local AST navigation; graph relationships remain in harness_explore.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct VueExtractPreviewArgs {
    pub path: String,
    #[serde(rename = "expectedHash")]
    pub expected_hash: String,
    /// Current template element ID returned by Vue outline.
    #[serde(rename = "symbolId")]
    pub symbol_id: String,
    #[serde(rename = "targetPath")]
    pub target_path: String,
    #[serde(rename = "componentName")]
    pub component_name: String,
    #[serde(alias = "sessionId")]
    pub session: Option<String>,
}

/// Local AST navigation and read-only Vue extraction planning.
/// - `read`: REQUIRED `{action: "read", path: string, symbol: string}`. OPTIONAL `{container?, symbolId?, view?: "full"|"signature", session?}`.
/// - `outline`: REQUIRED `{action: "outline", path: string}`. OPTIONAL `{query?, filterKind?: "method", view?: "compact", maxChars?, session?}`.
/// - `extract_preview`: REQUIRED `{action: "extract_preview", path, expectedHash, symbolId, targetPath, componentName}`.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SymbolsArgs {
    Outline(OutlineArgs),
    Read(ReadSymbolArgs),
    ExtractPreview(VueExtractPreviewArgs),
}

impl SymbolsArgs {
    pub fn into_request(self) -> Request {
        match self {
            Self::Outline(args) => Request::Outline(args),
            Self::Read(args) => Request::ReadSymbol(args),
            Self::ExtractPreview(args) => Request::VueExtractPreview(args),
        }
    }
}
