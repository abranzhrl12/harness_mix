//! Use-case orchestration. Depends on domain and ports, never on MCP or OS adapters.
mod admission;
mod batch;
mod dispatch;
pub mod indexing;
mod logging;
mod metrics;
pub mod ports;
mod reading_guidance;
mod session;

use harness_domain::{Failure, contracts::Request, execution::OperationContext, results::Results};
use ports::{SessionRepository, Workspace};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub struct Application<W, S> {
    workspace: Arc<W>,
    sessions: Arc<Mutex<S>>,
    results: Arc<Mutex<Results>>,
    admission: Arc<admission::Admission>,
    metrics: Arc<metrics::Metrics>,
    reading_guidance: Arc<Mutex<reading_guidance::Guidance>>,
    tool_profile: harness_domain::tool_contract::ToolProfile,
}

/// Owns admission before it is submitted to any worker pool; drop always frees it.
pub struct Operation<W, S> {
    application: Application<W, S>,
    request: Request,
    context: OperationContext,
    measurement: metrics::Measurement,
    _permit: admission::Permit,
}
impl<W: Workspace, S: SessionRepository> Operation<W, S> {
    pub fn request_id(&self) -> &str {
        self.measurement.request_id()
    }
    pub fn run(mut self) -> Value {
        let mut output = self
            .application
            .execute_admitted(self.request, &self.context);
        output["requestId"] = json!(self.measurement.request_id());
        self.measurement.finish_envelope(&output);
        if let Some(archive) = self.application.metrics.logs.archive() {
            let sequence = archive["lastSequence"].as_u64().unwrap_or(0);
            let saved = self.application.workspace.persist_log_archive(&archive);
            let saved_through = saved.as_ref().ok().copied();
            self.application.metrics.logs.saved(
                saved_through.unwrap_or(sequence),
                saved.as_ref().err().map(|f| f.0),
            );
            output["logPersistence"] = json!({"saved":saved.is_ok(),"requestedSequence":sequence,"savedThrough":saved_through});
            if let Err((code, _)) = saved {
                output["loggingWarning"] = json!({"code":code,"toolOutcomeUnchanged":true});
            }
        }
        output
    }
}

struct SessionLease<S: SessionRepository> {
    sessions: Arc<Mutex<S>>,
    id: String,
}
impl<S: SessionRepository> Drop for SessionLease<S> {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.release(&self.id);
        }
    }
}
impl<W, S> Clone for Application<W, S> {
    fn clone(&self) -> Self {
        Self {
            workspace: self.workspace.clone(),
            sessions: self.sessions.clone(),
            results: self.results.clone(),
            admission: self.admission.clone(),
            metrics: self.metrics.clone(),
            reading_guidance: self.reading_guidance.clone(),
            tool_profile: self.tool_profile,
        }
    }
}
impl<W: Workspace, S: SessionRepository> Application<W, S> {
    pub fn new(workspace: W, sessions: S) -> Self {
        let enabled = workspace.log_persistence_enabled();
        let history = if enabled {
            workspace.load_log_archive()
        } else {
            Ok(None)
        };
        let logs = logging::Log::configured(enabled, history);
        Self {
            workspace: Arc::new(workspace),
            sessions: Arc::new(Mutex::new(sessions)),
            results: Arc::default(),
            admission: Arc::default(),
            metrics: Arc::new(metrics::Metrics::with_logs(logs)),
            reading_guidance: Arc::default(),
            tool_profile: harness_domain::tool_contract::ToolProfile::Full,
        }
    }
    pub fn with_tool_profile(
        mut self,
        profile: harness_domain::tool_contract::ToolProfile,
    ) -> Self {
        self.tool_profile = profile;
        self
    }
    pub fn tool_profile(&self) -> harness_domain::tool_contract::ToolProfile {
        self.tool_profile
    }
    /// A single boundary for every transport: session admission, execution and envelope.
    pub fn execute(&self, request: Request) -> Value {
        let context = OperationContext::with_timeout(request.execution_timeout());
        self.execute_with_context(request, context)
    }
    pub fn execute_with_context(&self, request: Request, context: OperationContext) -> Value {
        match self.prepare(request, context) {
            Ok(operation) => operation.run(),
            Err(envelope) => envelope,
        }
    }
    pub fn prepare(
        &self,
        request: Request,
        context: OperationContext,
    ) -> Result<Operation<W, S>, Value> {
        let request_id = harness_domain::execution::new_request_id();
        if let Err(failure) = self.tool_profile.check(&request) {
            self.metrics
                .rejected_with_id(request.name(), failure.0, &request_id);
            let mut output = self.failure(request.name(), &request.arguments(), failure);
            output["requestId"] = json!(request_id);
            return Err(output);
        }
        let permit = context
            .check()
            .and_then(|()| {
                if matches!(&request, Request::Batch(_)) {
                    self.admission.acquire_batch()
                } else {
                    self.admission
                        .acquire(matches!(&request, Request::Search(_)))
                }
            })
            .map_err(|failure| {
                self.metrics
                    .rejected_with_id(request.name(), failure.0, &request_id);
                let mut output = self.failure(request.name(), &request.arguments(), failure);
                output["requestId"] = json!(request_id);
                output
            })?;
        let measurement = self.metrics.admitted_with_id(request.name(), &request_id);
        Ok(Operation {
            application: self.clone(),
            request,
            context,
            _permit: permit,
            measurement,
        })
    }
    pub fn failure(&self, name: &str, args: &Value, failure: Failure) -> Value {
        // Rejections/cancellation must not block the async transport on a worker's store lock.
        Results::default().envelope(name, args, Err(failure))
    }
    fn output(&self, name: &str, args: &Value, result: Result<Value, Failure>) -> Value {
        match self.results.lock() {
            Ok(mut results) => results.envelope(name, args, result),
            Err(_) => Results::default().envelope(
                name,
                args,
                Err(("INTERNAL", "almacén de resultados no disponible")),
            ),
        }
    }
}
