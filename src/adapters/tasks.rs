//! Native composition facade for the reusable background job engine.
use harness_application::ports::{JobEventStream, JobFuture, JobService};
use harness_domain::{
    Failure, assistance::ObserveArgs, contracts::ProcessStatusArgs, tasks::TaskArgs,
};
use serde_json::Value;
use std::{path::Path, sync::Arc, time::Duration};

/// Compatibility entry point that injects native verification and path protection.
#[derive(Clone)]
pub struct TaskManager(harness_jobs::TaskManager);

impl TaskManager {
    pub fn new(root: &Path) -> std::io::Result<Self> {
        harness_jobs::TaskManager::new(
            root,
            Arc::new(super::task_execution::NativeVerificationExecutor),
            Arc::new(super::task_pinning::NativeDirectoryPinner),
        )
        .map(Self)
    }

    pub fn diagnostics(&self) -> Value {
        self.0.diagnostics()
    }

    pub fn process_status(&self, args: &ProcessStatusArgs) -> Result<Value, Failure> {
        self.0.process_status(args)
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Value> {
        self.0.subscribe()
    }

    pub fn shutdown(&self) {
        self.0.shutdown();
    }

    pub async fn shutdown_and_wait(&self, timeout: Duration) -> bool {
        self.0.shutdown_and_wait(timeout).await
    }

    pub async fn handle(&self, args: TaskArgs) -> Result<Value, Failure> {
        self.0.handle(args).await
    }

    pub fn observe(&self, args: &ObserveArgs) -> Result<Value, Failure> {
        self.0.observe(args)
    }
}

impl JobService for TaskManager {
    fn handle(&self, args: TaskArgs) -> JobFuture<'_> {
        JobService::handle(&self.0, args)
    }

    fn subscribe(&self) -> Box<dyn JobEventStream> {
        JobService::subscribe(&self.0)
    }
}
