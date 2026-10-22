//! Application port implementation for the in-memory session repository.
//! Snapshot persistence remains in `sessions::persistence`; this adapter maps
//! the application session contract onto lifecycle and workflow operations.
use super::Sessions;
use harness_application::ports::{SessionContext, SessionRepository};
use harness_domain::Failure;
use serde_json::Value;

impl SessionRepository for Sessions {
    fn invalidate_checks(&mut self, id: &str, observation: &Value) -> Result<(), Failure> {
        harness_domain::freshness::invalidate_observed_checks(
            &mut self.get(id)?.observed_checks,
            observation,
        );
        Ok(())
    }
    fn workflow_gate(&mut self, id: &str) -> Result<std::sync::Arc<std::sync::Mutex<()>>, Failure> {
        Ok(self.get(id)?.workflow_gate.clone())
    }
    fn tasks(
        &mut self,
        args: &harness_domain::contracts::TasksArgs,
    ) -> Result<harness_application::ports::SessionMutation, Failure> {
        self.workflow_tasks(args)
    }
    fn checkpoint(
        &mut self,
        args: &harness_domain::contracts::CheckpointArgs,
    ) -> Result<harness_application::ports::SessionMutation, Failure> {
        self.workflow_checkpoint(args)
    }
    fn snapshot(&mut self, id: &str) -> Result<Value, Failure> {
        self.session_snapshot(id)
    }
    fn record(
        &mut self,
        id: &str,
        tool: &str,
        args: &Value,
        result: &Value,
    ) -> Result<Option<Value>, Failure> {
        self.record_observed(id, tool, args, result)
    }
    fn resolve_id(&mut self, id: &str) -> Result<String, Failure> {
        self.resolve_id(id)
    }
    fn acquire(&mut self, id: &str) -> Result<(), Failure> {
        self.acquire(id)
    }
    fn release(&mut self, id: &str) {
        self.release(id);
    }
    fn open(&mut self, id: Option<&str>) -> Result<SessionContext, Failure> {
        let (id, warning) = match id {
            Some("new") | None => self.create()?,
            Some(spec) => {
                let resolved = self.resolve_id(spec)?;
                (resolved, None)
            }
        };
        let session = self.get(&id)?;
        Ok(SessionContext {
            id,
            restored: session.restored,
            rules_hash: session.rules_hash.clone(),
            capabilities_hash: session.capabilities_hash.clone(),
            task_summary: session.summary(),
            persistence_warning: warning,
        })
    }
    fn update_hashes(
        &mut self,
        id: &str,
        rules: String,
        capabilities: String,
    ) -> Result<(), Failure> {
        let session = self.get(id)?;
        session.rules_hash = Some(rules);
        session.capabilities_hash = Some(capabilities);
        Ok(())
    }
}
