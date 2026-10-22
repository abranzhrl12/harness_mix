use crate::adapters::NativeWorkspace;
use harness_application::ports::workspace::SessionPersistence;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

impl SessionPersistence for NativeWorkspace {
    fn handover(
        &self,
        args: &harness_domain::contracts::HandoverArgs,
        snapshot: &Value,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.session_service.handover(args, snapshot, context)
    }
    fn persist_session(
        &self,
        id: &str,
        snapshot: &Value,
        context: &OperationContext,
    ) -> Result<(), Failure> {
        self.session_service.persist_session(id, snapshot, context)
    }
}
