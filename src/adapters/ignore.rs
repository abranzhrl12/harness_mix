//! Manage `.harnessignore` policy files.

mod operations;
mod policy;
mod storage;

#[cfg(test)]
mod tests;

use super::project::Project;

pub(super) const FILE: &str = ".harnessignore";
pub(super) const DEFAULT_EXCLUSIONS: &[&str] = &["node_modules/", "target/"];
pub(super) const HARD_EXCLUSIONS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".env*",
    "credential*",
    "secret*",
    "id_rsa",
    "id_ed25519",
    "oauth_creds.json",
    "mcp_config.json",
];
pub(super) const MAX_PATTERNS: usize = policy::MAX_PATTERNS;

use harness_domain::{Failure, contracts::IgnoreArgs, execution::OperationContext};
use serde_json::Value;

pub(super) fn ignore(
    project: &Project,
    args: &IgnoreArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    operations::ignore(project, args, context)
}

/// Effective pattern lines for one policy file, used by search reporting.
pub(super) fn pattern_count(project: &Project, relative_scope: &str) -> Option<usize> {
    let directory = project.target(relative_scope).ok()?;
    let file = directory.join(FILE);
    storage::read_policy(&file)
        .ok()
        .flatten()
        .map(|(_, patterns, _)| patterns)
}
