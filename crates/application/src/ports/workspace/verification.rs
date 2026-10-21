use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

/// Bounded verification using approved plans.
pub trait Verification: Send + Sync {
    fn verify(
        &self,
        _args: &harness_domain::contracts::VerifyArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Verification adapter unavailable"))
    }
}
