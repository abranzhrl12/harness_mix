use crate::adapters::{
    NativeWorkspace, diagnostics, doctor, fingerprint, process_status, sessions,
};
use harness_application::ports::workspace::WorkspaceDiagnostics;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;
use serde_json::json;

impl WorkspaceDiagnostics for NativeWorkspace {
    fn doctor(
        &self,
        args: &harness_domain::contracts::DoctorArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let mut result = doctor::doctor(&self.project, args, context)?;
        let details = self
            .index_watch
            .as_ref()
            .and_then(|status| status.lock().ok().map(|v| v.clone()))
            .unwrap_or_else(|| json!({"enabled":false,"state":"disabled"}));
        doctor::append_check(
            &mut result,
            json!({
                "id":"index-auto-sync",
                "status":if !details["lastError"].is_null() { "warning" } else if details["enabled"] != true { "skipped" } else { "ok" },
                "message":"Automatic index maintenance status; global freshness not certified",
                "details":details
            }),
        );
        match &self.tasks {
            Some(tasks) => {
                let diagnostics = tasks.diagnostics();
                doctor::append_check(
                    &mut result,
                    json!({
                        "id":"background-jobs", "status":diagnostics["status"],
                        "message":"Shared job registry and existing durable history inspected; no storage created or write probe performed",
                        "details":diagnostics
                    }),
                );
            }
            None => doctor::append_check(
                &mut result,
                json!({
                    "id":"background-jobs", "status":"skipped",
                    "message":"Background job manager not attached in this application/profile"
                }),
            ),
        }
        Ok(result)
    }
    fn process_status(
        &self,
        args: &harness_domain::contracts::ProcessStatusArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        process_status::handle_process_status_with_tasks(args, context, self.tasks.as_ref())
    }
    fn diagnostics(
        &self,
        args: &harness_domain::contracts::DiagnosticsArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        diagnostics::handle_diagnostics(&self.project, args, context)
    }
    fn observe_workspace(&self, context: &OperationContext) -> Result<Value, Failure> {
        let captured = fingerprint::capture(&self.project, context);
        context.check()?;
        let time = sessions::now();
        Ok(match captured {
            Ok(hash) => json!({"status":"captured","hash":hash,"capturedAt":time,
                "coverage":fingerprint::coverage()}),
            Err((code, _)) => json!({"status":"unknown","error":code,"capturedAt":time,
                "coverage":fingerprint::coverage()}),
        })
    }
}
