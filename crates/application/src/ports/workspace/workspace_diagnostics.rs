use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

/// Workspace diagnostics and bounded observations.
pub trait WorkspaceDiagnostics: Send + Sync {
    fn doctor(
        &self,
        _args: &harness_domain::contracts::DoctorArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Doctor adapter unavailable"))
    }
    fn process_status(
        &self,
        _args: &harness_domain::contracts::ProcessStatusArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Process status adapter unavailable"))
    }
    fn diagnostics(
        &self,
        _args: &harness_domain::contracts::DiagnosticsArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Diagnostics adapter unavailable"))
    }
    /// Bounded filesystem observation; called outside the session repository mutex.
    fn observe_workspace(&self, _context: &OperationContext) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Workspace observation adapter unavailable"))
    }
}
