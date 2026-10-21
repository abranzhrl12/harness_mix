use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

/// Session snapshots and handover persistence.
pub trait SessionPersistence: Send + Sync {
    fn handover(
        &self,
        _args: &harness_domain::contracts::HandoverArgs,
        _snapshot: &Value,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Handover adapter unavailable"))
    }
    fn persist_session(
        &self,
        _id: &str,
        _snapshot: &Value,
        _context: &OperationContext,
    ) -> Result<(), Failure> {
        Err(("PERSISTENCE", "Session persistence adapter unavailable"))
    }
}
