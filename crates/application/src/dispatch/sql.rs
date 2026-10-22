use crate::{
    Application, SessionLease,
    ports::{SessionRepository, Workspace},
};
use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

/// All SQL operations require the same session admission before touching the port.
pub(super) fn with_session<W, S, F>(
    app: &Application<W, S>,
    session: Option<&str>,
    context: &OperationContext,
    lease: &mut Option<SessionLease<S>>,
    operation: F,
) -> Result<Value, Failure>
where
    W: Workspace,
    S: SessionRepository,
    F: FnOnce(&W, &OperationContext) -> Result<Value, Failure>,
{
    app.lease(session, lease)
        .and_then(|()| operation(&app.workspace, context))
}
