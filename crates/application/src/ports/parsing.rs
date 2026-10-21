use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

/// A syntax validation strategy selected by the file path.
///
/// Implementations are immutable and may be shared across concurrent operations.
pub trait SyntaxParser: Send + Sync {
    fn supports(&self, path: &str) -> bool;

    fn validate(
        &self,
        path: &str,
        source: &str,
        context: &OperationContext,
    ) -> Result<Value, Failure>;
}
