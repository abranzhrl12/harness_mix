//! Native implementations of application ports. No MCP handlers or schemas here.
mod index_razor_mvc;
mod parsers;
mod vue_extract;
mod workspace;
use parsers::{
    astro::syntax as astro_syntax, csharp::syntax as csharp_syntax,
    ecmascript::symbols as ts_symbols, ecmascript::syntax as ts_syntax, razor::mvc as razor_mvc,
    razor::render as razor_render, razor::syntax as razor_syntax, rust::symbols as rust_symbols,
    rust::syntax as rust_syntax, vue::structure as vue_structure, vue::syntax as vue_syntax,
};
mod capabilities;
mod config;
mod create_file;
mod csproj;
mod disk_logs;
mod doctor;
mod edit;
mod edit_commit;
mod edit_security;
mod explore;
mod external_scopes;
mod fingerprint;
mod git;
mod ignore;
pub mod index_extract;
mod index_freshness;
mod index_inventory;
#[cfg(windows)]
mod index_semantic;
#[cfg(windows)]
pub mod index_store;
#[cfg(windows)]
pub(crate) mod index_watch;
mod move_file;
#[cfg(not(windows))]
pub mod index_store {
    pub fn sync_workspace(
        _: &std::path::Path,
        _: &harness_domain::execution::OperationContext,
    ) -> Result<serde_json::Value, harness_domain::Failure> {
        Err((
            "UNSUPPORTED",
            "Native index publication requires Windows path pinning",
        ))
    }
    pub fn sync_file(
        _: &std::path::Path,
        _: &str,
        _: &harness_domain::execution::OperationContext,
    ) -> Result<serde_json::Value, harness_domain::Failure> {
        Err((
            "UNSUPPORTED",
            "Native index publication requires Windows path pinning",
        ))
    }
}
pub mod data_query;
pub mod database_setup;
pub mod diagnostics;
pub mod handover;
pub mod index_sync;
mod postgresql;
pub mod process;
pub mod process_status;
mod process_tree;
mod project;
mod search;
mod session_workflow;
mod sessions;
pub mod sql;
mod sql_credentials;
mod sql_discovery;
mod sql_map;
mod sql_policy;
mod sql_schema_cache;
pub mod svg;
mod symbol_graph;
mod symbols;
pub mod tasks;
mod tasks_store;
mod verify;
pub mod xlsx;

use harness_application::{
    Application,
    ports::{SessionContext, SessionRepository},
};
use harness_domain::{Failure, execution::OperationContext};
use project::Project;
use serde_json::{Value, json};
use std::path::Path;

pub(super) const INDEX_BUSY_TIMEOUT_MS: u64 = 5_000;

pub struct NativeWorkspace {
    external_scopes: std::sync::Mutex<external_scopes::Registry>,
    log_store: Option<Result<disk_logs::DiskLogs, Failure>>,
    project: Project,
    edit_lock: std::sync::Mutex<()>,
    tasks: Option<tasks::TaskManager>,
    index_watch: Option<std::sync::Arc<std::sync::Mutex<Value>>>,
}
pub type NativeApplication = Application<NativeWorkspace, sessions::Sessions>;

pub fn application(root: &Path) -> std::io::Result<NativeApplication> {
    application_with_logging(root, false)
}
pub fn application_with_logging(
    root: &Path,
    persist_logs: bool,
) -> std::io::Result<NativeApplication> {
    application_with_logging_and_tasks(root, persist_logs, None)
}

pub fn application_with_logging_and_tasks(
    root: &Path,
    persist_logs: bool,
    tasks: Option<tasks::TaskManager>,
) -> std::io::Result<NativeApplication> {
    application_with_background(root, persist_logs, tasks, None)
}

pub(crate) fn application_with_background(
    root: &Path,
    persist_logs: bool,
    tasks: Option<tasks::TaskManager>,
    index_watch: Option<std::sync::Arc<std::sync::Mutex<Value>>>,
) -> std::io::Result<NativeApplication> {
    let project = Project::new(root)?;
    let sessions = sessions::Sessions::new(project.clone());
    let log_store = persist_logs.then(|| disk_logs::DiskLogs::new(project.clone()));
    Ok(Application::new(
        NativeWorkspace {
            external_scopes: std::sync::Mutex::default(),
            log_store,
            project,
            edit_lock: std::sync::Mutex::new(()),
            tasks,
            index_watch,
        },
        sessions,
    ))
}

impl NativeWorkspace {
    fn read_project(
        &self,
        scope: Option<&str>,
        session: Option<&str>,
        context: &OperationContext,
        read: impl FnOnce(&Project, Option<&str>) -> Result<Value, Failure>,
    ) -> Result<Value, Failure> {
        let Some(id) = scope else {
            return read(&self.project, None);
        };
        if !harness_domain::execution::valid_request_id(id) {
            return Err(("EXTERNAL_SCOPE", "Invalid external scope ID"));
        }
        let session = session.ok_or((
            "EXTERNAL_SCOPE",
            "externalScope requires its owning session",
        ))?;
        let grant = self
            .external_scopes
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
}

impl SessionRepository for sessions::Sessions {
    fn invalidate_checks(&mut self, id: &str, observation: &Value) -> Result<(), Failure> {
        harness_domain::freshness::invalidate_observed_checks(
            &mut self.get(id)?.observed_checks,
            observation,
        );
        Ok(())
    }
    fn workflow_gate(&mut self, id: &str) -> Result<std::sync::Arc<std::sync::Mutex<()>>, Failure> {
        Ok(self.get(id)?.workflow_gate.clone())
    }
    fn tasks(
        &mut self,
        args: &harness_domain::contracts::TasksArgs,
    ) -> Result<harness_application::ports::SessionMutation, Failure> {
        self.workflow_tasks(args)
    }
    fn checkpoint(
        &mut self,
        args: &harness_domain::contracts::CheckpointArgs,
    ) -> Result<harness_application::ports::SessionMutation, Failure> {
        self.workflow_checkpoint(args)
    }
    fn snapshot(&mut self, id: &str) -> Result<Value, Failure> {
        self.session_snapshot(id)
    }
    fn record(
        &mut self,
        id: &str,
        tool: &str,
        args: &Value,
        result: &Value,
    ) -> Result<Option<Value>, Failure> {
        self.record_observed(id, tool, args, result)
    }
    fn resolve_id(&mut self, id: &str) -> Result<String, Failure> {
        self.resolve_id(id)
    }
    fn acquire(&mut self, id: &str) -> Result<(), Failure> {
        self.acquire(id)
    }
    fn release(&mut self, id: &str) {
        self.release(id);
    }
    fn open(&mut self, id: Option<&str>) -> Result<SessionContext, Failure> {
        let (id, warning) = match id {
            Some("new") | None => self.create()?,
            Some(spec) => {
                let resolved = self.resolve_id(spec)?;
                (resolved, None)
            }
        };
        let session = self.get(&id)?;
        Ok(SessionContext {
            id,
            restored: session.restored,
            rules_hash: session.rules_hash.clone(),
            capabilities_hash: session.capabilities_hash.clone(),
            task_summary: session.summary(),
            persistence_warning: warning,
        })
    }
    fn update_hashes(
        &mut self,
        id: &str,
        rules: String,
        capabilities: String,
    ) -> Result<(), Failure> {
        let session = self.get(id)?;
        session.rules_hash = Some(rules);
        session.capabilities_hash = Some(capabilities);
        Ok(())
    }
}
