use harness_domain::{Failure, contracts::SearchArgs, execution::OperationContext};
use serde_json::Value;

use crate::ports::workspace::SourceQueries;

/// Run a source search using the read-only query capability.
pub fn handle<S: SourceQueries + ?Sized>(
    service: &S,
    args: &SearchArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    service.search(args, context)
}
