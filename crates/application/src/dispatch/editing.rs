use crate::{
    Application, SessionLease,
    ports::{SessionRepository, Workspace},
};
use harness_domain::{
    Failure,
    contracts::{IgnoreArgs, MoveArgs, ReplaceArgs, VerifyArgs, WriteArgs},
    execution::OperationContext,
};
use serde_json::Value;

fn recorded_session_write<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    session: Option<&str>,
    lease: &mut Option<SessionLease<S>>,
    context: &OperationContext,
    name: &str,
    args: &Value,
    write: impl FnOnce() -> Result<Value, Failure>,
) -> Result<Value, Failure> {
    app.with_session_write(session, lease, context, |session_id| {
        let mut result = write()?;
        app.record_result(session_id, name, args, &mut result);
        Ok(result)
    })
}

pub(super) fn delete_file<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: harness_domain::contracts::DeleteArgs,
    context: &OperationContext,
    name: &str,
    request_arguments: &Value,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    recorded_session_write(
        app,
        args.session.as_deref(),
        lease,
        context,
        name,
        request_arguments,
        || crate::handlers::edit::delete(app.workspace.as_ref(), &args, context),
    )
}

pub(super) fn replace<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: ReplaceArgs,
    context: &OperationContext,
    name: &str,
    request_arguments: &Value,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    if args.preview {
        app.lease(args.session.as_deref(), lease).and_then(|()| {
            crate::handlers::edit::patch_preview(app.workspace.as_ref(), &args, context)
        })
    } else {
        recorded_session_write(
            app,
            args.session.as_deref(),
            lease,
            context,
            name,
            request_arguments,
            || crate::handlers::edit::replace(app.workspace.as_ref(), &args, context),
        )
    }
}

pub(super) fn write<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: WriteArgs,
    context: &OperationContext,
    name: &str,
    request_arguments: &Value,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    recorded_session_write(
        app,
        args.session.as_deref(),
        lease,
        context,
        name,
        request_arguments,
        || crate::handlers::edit::write(app.workspace.as_ref(), &args, context),
    )
}

pub(super) fn move_file<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: MoveArgs,
    context: &OperationContext,
    name: &str,
    request_arguments: &Value,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    recorded_session_write(
        app,
        args.session.as_deref(),
        lease,
        context,
        name,
        request_arguments,
        || crate::handlers::edit::move_file(app.workspace.as_ref(), &args, context),
    )
}

pub(super) fn ignore<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: IgnoreArgs,
    context: &OperationContext,
    name: &str,
    request_arguments: &Value,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    if args.action == harness_domain::contracts::IgnoreAction::List {
        app.lease(args.session.as_deref(), lease)
            .and_then(|()| crate::handlers::edit::ignore(app.workspace.as_ref(), &args, context))
    } else {
        recorded_session_write(
            app,
            args.session.as_deref(),
            lease,
            context,
            name,
            request_arguments,
            || crate::handlers::edit::ignore(app.workspace.as_ref(), &args, context),
        )
    }
}

pub(super) fn verify<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: VerifyArgs,
    context: &OperationContext,
    name: &str,
    request_arguments: &Value,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    recorded_session_write(
        app,
        args.session.as_deref(),
        lease,
        context,
        name,
        request_arguments,
        || crate::handlers::verify::handle(app.workspace.as_ref(), &args, context),
    )
}
