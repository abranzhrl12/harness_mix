use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

pub mod agent;
pub mod browser;
pub mod conversation;
pub mod indexing;
pub mod jobs;
pub mod parsing;
pub use jobs::{JobEvent, JobEventFuture, JobEventStream, JobFuture, JobService};

/// Platform-dependent I/O. Implementations enforce filesystem safety and scan budgets.
pub use workspace::Workspace;
pub mod workspace;

/// Internal execution only; callers validate tool-specific arguments and scope.
/// Implementations drain both pipes concurrently and enforce a shared byte budget.
/// Admission must remain held until the child has been reaped.
pub trait ProcessRunner: Send + Sync {
    fn run(
        &self,
        spec: &harness_domain::process::ProcessSpec,
        context: &OperationContext,
    ) -> Result<harness_domain::process::ProcessOutcome, Failure>;
}

pub struct SessionContext {
    pub id: String,
    pub restored: bool,
    pub rules_hash: Option<String>,
    pub capabilities_hash: Option<String>,
    pub task_summary: Value,
    pub persistence_warning: Option<Value>,
}

/// The application decides when to open/cache a session; the adapter owns storage.
pub trait SessionRepository: Send {
    /// Return the same gate for a pinned session throughout its cache lifetime.
    fn workflow_gate(&mut self, id: &str) -> Result<std::sync::Arc<std::sync::Mutex<()>>, Failure>;
    fn tasks(
        &mut self,
        _args: &harness_domain::contracts::TasksArgs,
    ) -> Result<SessionMutation, Failure> {
        Err(("UNSUPPORTED", "Tasks adapter unavailable"))
    }
    fn checkpoint(
        &mut self,
        _args: &harness_domain::contracts::CheckpointArgs,
    ) -> Result<SessionMutation, Failure> {
        Err(("UNSUPPORTED", "Checkpoint adapter unavailable"))
    }
    fn snapshot(&mut self, _id: &str) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Session snapshot adapter unavailable"))
    }
    /// Retain negative freshness facts in live checks; never revive historical checks.
    fn invalidate_checks(&mut self, _id: &str, _observation: &Value) -> Result<(), Failure> {
        Ok(())
    }
    fn record(
        &mut self,
        _id: &str,
        _tool: &str,
        _args: &Value,
        _result: &Value,
    ) -> Result<Option<Value>, Failure> {
        Ok(None)
    }
    fn resolve_id(&mut self, id: &str) -> Result<String, Failure> {
        Ok(id.to_string())
    }
    fn acquire(&mut self, id: &str) -> Result<(), Failure>;
    fn release(&mut self, id: &str);
    fn open(&mut self, id: Option<&str>) -> Result<SessionContext, Failure>;
    fn update_hashes(
        &mut self,
        id: &str,
        rules: String,
        capabilities: String,
    ) -> Result<(), Failure>;
}

/// Owned snapshot lets the workspace persist outside the global repository mutex.
pub struct SessionMutation {
    pub data: Value,
    pub snapshot: Option<Value>,
}
