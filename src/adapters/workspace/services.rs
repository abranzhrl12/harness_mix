//! Stateful workspace capabilities composed by `NativeWorkspace`.
use crate::adapters::{Project, external_scopes};
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[path = "services/sessions.rs"]
mod sessions;
#[path = "services/source_editing.rs"]
mod source_editing;
#[path = "services/source_queries.rs"]
mod source_queries;

pub(in crate::adapters) struct SourceQueryService {
    pub(in crate::adapters) project: Project,
    pub(in crate::adapters) external_scopes: Arc<Mutex<external_scopes::Registry>>,
    pub(in crate::adapters) edit_lock: Arc<Mutex<()>>,
}

impl SourceQueryService {
    pub(in crate::adapters) fn new(
        project: Project,
        external_scopes: Arc<Mutex<external_scopes::Registry>>,
        edit_lock: Arc<Mutex<()>>,
    ) -> Self {
        Self {
            project,
            external_scopes,
            edit_lock,
        }
    }
}

pub(in crate::adapters) struct SourceEditingService {
    pub(in crate::adapters) project: Project,
    pub(in crate::adapters) edit_lock: Arc<Mutex<()>>,
}

impl SourceEditingService {
    pub(in crate::adapters) fn new(project: Project, edit_lock: Arc<Mutex<()>>) -> Self {
        Self { project, edit_lock }
    }
}

pub(in crate::adapters) struct SessionService {
    pub(in crate::adapters) project: Project,
}

impl SessionService {
    pub(in crate::adapters) fn new(project: Project) -> Self {
        Self { project }
    }
}

pub(in crate::adapters) fn read_project(
    scopes: &Arc<Mutex<external_scopes::Registry>>,
    id: &str,
    session: &str,
    context: &OperationContext,
    read: impl FnOnce(&Project, Option<&str>) -> Result<Value, Failure>,
) -> Result<Value, Failure> {
    let grant = scopes
        .lock()
        .map_err(|_| ("INTERNAL", "External registry unavailable"))?
        .lookup(session, id)?;
    let scoped = grant.resolve(context)?;
    let mut result = read(&scoped.project, scoped.default_scope.as_deref())?;
    let info = grant.describe();
    result["externalScope"] = json!(id);
    result["externalScopeInfo"] = info.clone();
    if let Some(detail) = result.get_mut("_detail").filter(|v| v.is_object()) {
        detail["externalScope"] = json!(id);
        detail["externalScopeInfo"] = info;
    }
    Ok(result)
}

pub(in crate::adapters) fn read_scoped_project(
    project: &Project,
    scopes: &Arc<Mutex<external_scopes::Registry>>,
    scope: Option<&str>,
    session: Option<&str>,
    context: &OperationContext,
    read: impl FnOnce(&Project, Option<&str>) -> Result<Value, Failure>,
) -> Result<Value, Failure> {
    match scope {
        Some(id) => {
            if !harness_domain::execution::valid_request_id(id) {
                return Err(("EXTERNAL_SCOPE", "Invalid external scope ID"));
            }
            let session = session.ok_or((
                "EXTERNAL_SCOPE",
                "externalScope requires its owning session",
            ))?;
            read_project(scopes, id, session, context, read)
        }
        None => read(project, None),
    }
}
