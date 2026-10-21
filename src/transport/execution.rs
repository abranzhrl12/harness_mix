//! Cancellation and admission ownership for blocking application operations.
use super::presentation::response;
use harness_application::{
    Application,
    ports::{SessionRepository, Workspace},
};
use harness_domain::{contracts::Request, execution::OperationContext};
use rmcp::model::CallToolResult;

struct CancelOnDrop(OperationContext);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(super) async fn execute<W: Workspace + 'static, S: SessionRepository + 'static>(
    application: &Application<W, S>,
    request: Request,
    cancelled: impl std::future::Future<Output = ()>,
) -> CallToolResult {
    let name = request.name();
    let mutating = request.is_mutating();
    let args = request.arguments();
    let timeout = request.execution_timeout();
    let verification = matches!(&request, Request::Verify(_));
    let context = OperationContext::with_timeout(timeout);
    // Dropping a handler (client cancellation/disconnect) also signals its worker.
    let _cancel_on_drop = CancelOnDrop(context.clone());
    let operation = match application.prepare(request, context.clone()) {
        Ok(operation) => operation,
        Err(envelope) => return response(envelope),
    };
    let request_id = operation.request_id().to_owned();
    let mut worker = tokio::task::spawn_blocking(move || operation.run());
    let mut envelope = tokio::select! {
        result = &mut worker => match result { Ok(envelope) => envelope, Err(_) => application.failure(name, &args, ("INTERNAL", "operación no completada; si era edición, releer antes de reintentar")) },
        () = cancelled => {
            context.cancel();
            if mutating { worker.await.unwrap_or_else(|_| application.failure(name, &args, ("EDIT_OUTCOME_UNKNOWN", "Releer archivo antes de reintentar"))) }
            else { application.failure(name, &args, ("CANCELLED", "operación cancelada")) }
        },
        () = tokio::time::sleep(timeout) => {
            context.cancel();
            if mutating || verification { worker.await.unwrap_or_else(|_| application.failure(name, &args, ("TIMEOUT", "Operation exceeded its deadline"))) }
            else { application.failure(name, &args, ("TIMEOUT", "operación superó el plazo")) }
        },
    };
    // A detached worker retains its admission/session guards until it really stops.
    envelope["requestId"] = serde_json::json!(request_id);
    response(envelope)
}
