use crate::adapters::{NativeWorkspace, handover, sessions};
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
        handover::handle_handover(&self.project, args, snapshot, context)
    }
    fn persist_session(
        &self,
        id: &str,
        snapshot: &Value,
        context: &OperationContext,
    ) -> Result<(), Failure> {
        sessions::persist_snapshot(&self.project, id, snapshot, context)
    }
}
