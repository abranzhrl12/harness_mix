use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

/// Workspace identity, declared capabilities and access configuration.
pub trait WorkspaceMetadata: Send + Sync {
    fn external_scope(
        &self,
        _args: &harness_domain::contracts::ExternalScopeArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "External scope adapter unavailable"))
    }
    fn config(
        &self,
        _args: &harness_domain::contracts::ConfigArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Configuration section adapter unavailable"))
    }
    fn info(&self) -> Value;
    fn rules(&self, context: &OperationContext) -> Result<String, Failure>;
    fn capabilities(&self) -> Value;
}
