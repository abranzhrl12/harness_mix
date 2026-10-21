use harness_domain::{
    Failure,
    contracts::{ReadArgs, SearchArgs},
    execution::OperationContext,
};
use serde_json::Value;

/// Read-only source and graph queries; callers need no mutation or database ports.
pub trait SourceQueries: Send + Sync {
    fn explore(
        &self,
        _args: &harness_domain::contracts::ExploreArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Graph exploration adapter unavailable"))
    }
    fn outline(
        &self,
        _args: &harness_domain::contracts::OutlineArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Outline adapter unavailable"))
    }
    fn read_symbol(
        &self,
        _args: &harness_domain::contracts::ReadSymbolArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Symbol adapter unavailable"))
    }
    fn read(&self, args: &ReadArgs, context: &OperationContext) -> Result<Value, Failure>;
    fn search(&self, args: &SearchArgs, context: &OperationContext) -> Result<Value, Failure>;
}
