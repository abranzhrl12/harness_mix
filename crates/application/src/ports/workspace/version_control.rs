use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

/// Read-only version control observations.
pub trait VersionControl: Send + Sync {
    fn git_status(
        &self,
        _args: &harness_domain::contracts::GitStatusArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Git status adapter unavailable"))
    }
    fn git_session_changes(
        &self,
        _edits: &[Value],
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Ok(serde_json::json!([]))
    }
    fn git_diff(
        &self,
        _args: &harness_domain::contracts::GitDiffArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Git diff adapter unavailable"))
    }
}
