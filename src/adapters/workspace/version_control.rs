use crate::adapters::{NativeWorkspace, git};
use harness_application::ports::workspace::VersionControl;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

impl VersionControl for NativeWorkspace {
    fn git_status(
        &self,
        args: &harness_domain::contracts::GitStatusArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        git::status(&self.project, args, context)
    }
    fn git_session_changes(
        &self,
        edits: &[Value],
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        git::session_changes(&self.project, edits, context)
    }
    fn git_diff(
        &self,
        args: &harness_domain::contracts::GitDiffArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        git::diff(&self.project, args, context)
    }
}
