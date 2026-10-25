//! Policies shared by use-case orchestration, kept outside the individual handlers.
use harness_domain::{
    Failure, contracts::Request, execution::OperationContext, tool_contract::ToolProfile,
};
use serde_json::Value;

/// Creates the request-wide cooperative context with the request's configured deadline.
/// Progress observers and cancellation ownership remain on `OperationContext` itself.
pub(crate) fn context_for_request(request: &Request) -> OperationContext {
    OperationContext::with_timeout(request.execution_timeout())
}

/// Applies server-profile authorization before checking cancellation, preserving the
/// established rejection order. Admission limits are acquired by the caller afterward.
pub(crate) fn authorize(
    profile: ToolProfile,
    request: &Request,
    context: &OperationContext,
) -> Result<(), Failure> {
    profile.check(request)?;
    context.check()
}

/// Applies the common post-execution deadline check to successful read results.
/// Explicit partial search results survive cooperative cancellation/deadline; mutation
/// results are never reclassified after the adapter may have committed them.
pub(crate) fn finish(
    mutating: bool,
    value: Value,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let partial = value.get("cancelled") == Some(&Value::Bool(true))
        || value.get("timedOut") == Some(&Value::Bool(true));
    if !mutating && !partial {
        context.check()?;
    }
    Ok(value)
}
