//! Read-only Git adapter. No caller-selected command, shell, external diff or filters.

#[path = "git/diff.rs"]
mod diff;
#[path = "git/metadata.rs"]
mod metadata;
#[path = "git/reader.rs"]
mod reader;
#[path = "git/session_changes.rs"]
mod session_changes;
#[path = "git/status.rs"]
mod status;

use crate::adapters::project::Project;
use harness_domain::{
    Failure,
    contracts::{GitDiffArgs, GitStatusArgs},
    execution::OperationContext,
};
use serde_json::Value;

pub(super) fn status(
    project: &Project,
    args: &GitStatusArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    status::status(project, args, context)
}

pub(super) fn diff(
    project: &Project,
    args: &GitDiffArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    diff::diff(project, args, context)
}

pub(super) fn session_changes(
    project: &Project,
    edits: &[Value],
    context: &OperationContext,
) -> Result<Value, Failure> {
    session_changes::session_changes(project, edits, context)
}

#[cfg(test)]
#[path = "git/status_tests.rs"]
mod tests;
