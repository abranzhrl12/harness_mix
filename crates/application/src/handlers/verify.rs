use harness_domain::{Failure, contracts::VerifyArgs, execution::OperationContext};
use serde_json::Value;

use crate::ports::workspace::Verification;

/// Run verification with the verification capability only.
pub fn handle<V: Verification + ?Sized>(
    service: &V,
    args: &VerifyArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    service.verify(args, context)
}
