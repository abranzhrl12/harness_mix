//! Cancellation and admission ownership for blocking application operations.
use super::presentation::{clean_empty_metadata, compact_envelope_metadata, response_with_options};
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
    context_presentation: bool,
    structured_presentation: bool,
) -> CallToolResult {
    let compact = matches!(&request, Request::Read(args) if args.compact);
    let compact_batch_reads = match &request {
        Request::Batch(args) => args
            .jobs
            .iter()
            .enumerate()
            .filter_map(|(index, job)| {
                (job.tool.name() == "harness_read"
                    && job.arguments.get("compact") == Some(&serde_json::Value::Bool(true)))
                .then_some(index)
            })
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
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
        Err(envelope) => return response_with_options(envelope, compact, context_presentation),
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
    if let Some(results) = envelope
        .get_mut("data")
        .and_then(|data| data.get_mut("results"))
        .and_then(serde_json::Value::as_array_mut)
    {
        for result in results {
            clean_empty_metadata(result);
        }
    }
    if let Some(results) = envelope
        .get_mut("data")
        .and_then(|data| data.get_mut("_detail"))
        .and_then(|detail| detail.get_mut("results"))
        .and_then(serde_json::Value::as_array_mut)
    {
        for item in results {
            if let Some(result) = item.get_mut("result") {
                clean_empty_metadata(result);
            }
        }
    }
    for index in compact_batch_reads {
        if let Some(result) = envelope
            .get_mut("data")
            .and_then(|data| data.get_mut("results"))
            .and_then(serde_json::Value::as_array_mut)
            .and_then(|items| items.get_mut(index))
        {
            compact_envelope_metadata(result);
        }
        if let Some(result) = envelope
            .get_mut("data")
            .and_then(|data| data.get_mut("_detail"))
            .and_then(|detail| detail.get_mut("results"))
            .and_then(serde_json::Value::as_array_mut)
            .and_then(|items| items.get_mut(index))
            .and_then(|item| item.get_mut("result"))
        {
            compact_envelope_metadata(result);
        }
    }
    // A detached worker retains its admission/session guards until it really stops.
    envelope["requestId"] = serde_json::json!(request_id);
    if structured_presentation
        && matches!(
            name,
            "harness_bootstrap"
                | "harness_read"
                | "harness_symbols"
                | "harness_search"
                | "harness_batch"
                | "harness_explore"
        )
    {
        super::presentation::structured_presentation(envelope)
    } else {
        response_with_options(envelope, compact, context_presentation)
    }
}
