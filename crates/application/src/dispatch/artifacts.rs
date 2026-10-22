use crate::{
    Application, SessionLease,
    ports::{SessionRepository, Workspace},
};
use harness_domain::{
    Failure,
    contracts::{DataQueryArgs, DetailArgs},
    execution::OperationContext,
};
use serde_json::Value;

pub(super) fn data_query<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    mut args: DataQueryArgs,
    context: &OperationContext,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    app.lease(args.session.as_deref(), lease).and_then(|()| {
        if let Some(reference) = args.result_ref.take() {
            if args.path.is_some() || args.data.is_some() {
                return Err(("ARGUMENT", "Use exactly one of path, data or resultRef"));
            }
            args.data = Some(
                app.results
                    .lock()
                    .map_err(|_| ("INTERNAL", "Results unavailable"))?
                    .value(&reference)?,
            );
        }
        app.workspace.data_query(&args, context)
    })
}

pub(super) fn detail<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    args: DetailArgs,
    lease: &mut Option<SessionLease<S>>,
) -> Result<Value, Failure> {
    app.lease(args.session.as_deref(), lease).and_then(|()| {
        app.results
            .lock()
            .map_err(|_| ("INTERNAL", "almacén no disponible"))?
            .get(
                &args.reference,
                args.offset.unwrap_or(0),
                args.max_chars.unwrap_or(8000),
            )
    })
}
