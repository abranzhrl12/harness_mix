use crate::{
    Application, SessionLease,
    ports::{SessionRepository, Workspace},
};
use harness_domain::{
    Failure,
    contracts::{ExploreArgs, GitStatusArgs, OutlineArgs, ReadSymbolArgs},
    execution::OperationContext,
};
use serde_json::Value;

pub(super) fn explore<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: ExploreArgs,
    context: &OperationContext,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    app.lease(args.session.as_deref(), lease)
        .and_then(|()| crate::handlers::read::explore(app.workspace.as_ref(), &args, context))
}

pub(super) fn git_status<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: GitStatusArgs,
    context: &OperationContext,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    app.lease(args.session.as_deref(), lease).and_then(|()| {
        let mut result = app.workspace.git_status(&args, context)?;
        let mut edits = Vec::new();
        if let Some(id) = args.session.as_deref() {
            let snapshot = app
                .sessions
                .lock()
                .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                .snapshot(id)?;
            if let Some(previous) = snapshot["historical"]["modifiedFiles"].as_array() {
                edits.extend(previous.iter().cloned());
            }
            if let Some(observed) = snapshot["observedByHarness"]["modifiedFiles"].as_array() {
                edits.extend(observed.iter().cloned());
            }
        }
        result["sessionChanges"] = app.workspace.git_session_changes(&edits, context)?;
        Ok(result)
    })
}

pub(super) fn outline<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: OutlineArgs,
    context: &OperationContext,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    app.lease(args.session.as_deref(), lease)
        .and_then(|()| crate::handlers::read::outline(app.workspace.as_ref(), &args, context))
}

pub(super) fn read_symbol<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: ReadSymbolArgs,
    context: &OperationContext,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    app.lease(args.session.as_deref(), lease)
        .and_then(|()| crate::handlers::read::read_symbol(app.workspace.as_ref(), &args, context))
}
