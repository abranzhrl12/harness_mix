use crate::adapters::{NativeWorkspace, verify};
use harness_application::ports::workspace::Verification;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

impl Verification for NativeWorkspace {
    fn verify(
        &self,
        args: &harness_domain::contracts::VerifyArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        verify::verify(&self.project, args, context)
    }
}
