use crate::{
    Application, SessionLease,
    ports::{SessionRepository, Workspace},
};
use harness_domain::{
    Failure,
    contracts::{
        BootstrapArgs, CheckpointArgs, ExternalScopeArgs, HandoverArgs, TasksAction, TasksArgs,
    },
    execution::OperationContext,
};
use serde_json::Value;

pub(super) fn bootstrap<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: BootstrapArgs,
    context: &OperationContext,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    app.bootstrap(args, context, lease)
}

pub(super) fn external_scope<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: ExternalScopeArgs,
    context: &OperationContext,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    if matches!(
        args.action,
        harness_domain::contracts::ExternalScopeAction::List
    ) {
        app.lease(Some(&args.session), lease)
            .and_then(|()| app.workspace.external_scope(&args, context))
    } else {
        app.with_session_write(Some(&args.session), lease, context, |_| {
            app.workspace.external_scope(&args, context)
        })
    }
}

pub(super) fn handover<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: HandoverArgs,
    context: &OperationContext,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    let session = args.session.clone();
    if args.target_file.is_some() {
        app.with_session_write(Some(&session), lease, context, |session_id| {
            let real_id = session_id.unwrap_or(&session);
            let snapshot = app
                .sessions
                .lock()
                .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                .snapshot(real_id)?;
            app.workspace.handover(&args, &snapshot, context)
        })
    } else {
        app.lease(Some(&session), lease).and_then(|()| {
            let real_id = lease
                .as_ref()
                .map(|item| item.id.as_str())
                .unwrap_or(&session);
            let snapshot = app
                .sessions
                .lock()
                .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                .snapshot(real_id)?;
            app.workspace.handover(&args, &snapshot, context)
        })
    }
}

pub(super) fn tasks<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: TasksArgs,
    context: &OperationContext,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    let session = args.session.clone();
    if matches!(args.action, Some(TasksAction::Update)) {
        app.with_session_write(Some(&session), lease, context, |session_id| {
            let real_id = session_id.unwrap_or(&session);
            let update = app
                .sessions
                .lock()
                .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                .tasks(&args)?;
            Ok(app.persist_update(real_id, update))
        })
    } else {
        app.lease(Some(&session), lease).and_then(|()| {
            app.sessions
                .lock()
                .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                .tasks(&args)
                .map(|update| update.data)
        })
    }
}

pub(super) fn checkpoint<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: CheckpointArgs,
    context: &OperationContext,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    let session = args.session.clone();
    if args.has_updates() || args.refresh_freshness == Some(true) {
        app.with_session_write(Some(&session), lease, context, |session_id| {
            let real_id = session_id.unwrap_or(&session);
            // Observe outside the repository lock while the workflow gate is held.
            let observation = if args.refresh_freshness == Some(true) {
                Some(app.workspace.observe_workspace(context)?)
            } else {
                None
            };
            context.check()?;
            let mut sessions = app
                .sessions
                .lock()
                .map_err(|_| ("INTERNAL", "Sessions unavailable"))?;
            let mut update = sessions.checkpoint(&args)?;
            if let Some(observation) = &observation {
                sessions.invalidate_checks(real_id, observation)?;
            }
            drop(sessions);
            if let Some(observation) = observation {
                harness_domain::freshness::refresh_checkpoint(&mut update.data, &observation);
            }
            Ok(app.persist_update(real_id, update))
        })
    } else {
        app.lease(Some(&session), lease).and_then(|()| {
            let mut sessions = app
                .sessions
                .lock()
                .map_err(|_| ("INTERNAL", "Sessions unavailable"))?;
            let update = sessions.checkpoint(&args)?;
            drop(sessions);
            Ok(update.data)
        })
    }
}
