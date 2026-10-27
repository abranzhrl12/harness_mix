use super::Request;
use serde::{Deserialize, Serialize};

/// The application has one entry point; transports cannot bypass its admission checks.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GitStatusArgs {
    #[serde(rename = "includeUntracked")]
    pub include_untracked: Option<bool>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GitDiffArgs {
    #[serde(default = "workspace_path")]
    pub path: String,
    pub staged: Option<bool>,
    /// auto returns HEAD:index, index:worktree and HEAD:worktree separately.
    #[cfg_attr(feature = "schema", schemars(extend("enum" = ["auto", "HEAD:index", "index:worktree", "HEAD:worktree", null])))]
    pub comparison: Option<String>,
    #[serde(rename = "maxLines")]
    pub max_lines: Option<usize>,
    #[serde(rename = "maxChars")]
    pub max_chars: Option<usize>,
    pub session: Option<String>,
}

/// Read-only Git operations with action-specific arguments.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GitArgs {
    Status(GitStatusArgs),
    Diff(GitDiffArgs),
}

impl GitArgs {
    pub fn into_request(self) -> Request {
        match self {
            Self::Status(args) => Request::GitStatus(args),
            Self::Diff(args) => Request::GitDiff(args),
        }
    }
}

/// Omitted path diffs the whole workspace, same as an explicit ".".
fn workspace_path() -> String {
    ".".into()
}
