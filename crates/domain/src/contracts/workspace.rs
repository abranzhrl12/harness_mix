use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ConfigArgs {
    pub path: String,
    pub section: String,
    #[serde(rename = "maxChars")]
    pub max_chars: Option<usize>,
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    pub session: Option<String>,
    /// When true, extracts the raw section text with IIS/ASP.NET heuristic regex redaction
    /// (preserving keys and attributes, redacting sensitive secrets like password/pwd/token/connectionString).
    pub raw: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExternalScopeAction {
    Grant,
    List,
    Revoke,
}
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ExternalScopeArgs {
    pub action: ExternalScopeAction,
    pub session: String,
    pub root: Option<String>,
    #[serde(rename = "userAuthorized")]
    pub user_authorized: Option<bool>,
    #[serde(rename = "scopeId")]
    pub scope_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum IgnoreAction {
    List,
    Add,
    Remove,
    Init,
}

/// Manages `.harnessignore` policy files. Patterns use gitignore syntax.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct IgnoreArgs {
    pub action: IgnoreAction,
    /// Directory whose `.harnessignore` is managed; defaults to the workspace root.
    pub scope: Option<String>,
    /// Patterns for add/remove; ignored by list/init.
    pub patterns: Option<Vec<String>>,
    /// Optional note written as a comment above newly added patterns.
    pub reason: Option<String>,
    pub session: Option<String>,
}
