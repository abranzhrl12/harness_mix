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

pub(super) fn replace<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: ReplaceArgs,
    context: &OperationContext,
    name: &str,
    request_arguments: &Value,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    if args.preview {
        app.lease(args.session.as_deref(), lease)
            .and_then(|()| app.workspace.patch_preview(&args, context))
    } else {
        recorded_session_write(
            app,
            args.session.as_deref(),
            lease,
            context,
            name,
            request_arguments,
            || app.workspace.replace(&args, context),
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
        || app.workspace.write(&args, context),
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
        || app.workspace.move_file(&args, context),
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
            .and_then(|()| app.workspace.ignore(&args, context))
    } else {
        recorded_session_write(
            app,
            args.session.as_deref(),
            lease,
            context,
            name,
            request_arguments,
            || app.workspace.ignore(&args, context),
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
        || app.workspace.verify(&args, context),
    )
}
