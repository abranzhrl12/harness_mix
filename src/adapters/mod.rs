//! Native implementations of application ports. No MCP handlers or schemas here.
mod index_razor_mvc;
mod parsers;
mod vue_extract;
mod workspace;
pub mod workspace_set;
use parsers::{
    astro::syntax as astro_syntax, csharp::syntax as csharp_syntax,
    ecmascript::symbols as ts_symbols, ecmascript::syntax as ts_syntax, razor::mvc as razor_mvc,
    razor::render as razor_render, razor::syntax as razor_syntax, rust::symbols as rust_symbols,
    rust::syntax as rust_syntax, vue::structure as vue_structure, vue::syntax as vue_syntax,
};
pub mod acp;
pub mod agent;
mod capabilities;
mod composition;
mod config;
mod create_file;
mod csproj;
mod delete_file;
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
mod index_lock;
mod index_profile;
#[cfg(windows)]
mod index_semantic;
#[cfg(windows)]
pub mod index_store;
#[cfg(windows)]
pub(crate) mod index_watch;
mod move_file;
mod proposals;
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
mod task_execution;
mod task_pinning;
mod verify;
pub mod xlsx;

use harness_application::Application;
use harness_domain::{Failure, execution::OperationContext};
use project::Project;
use serde_json::Value;
use std::path::Path;

pub(super) const INDEX_BUSY_TIMEOUT_MS: u64 = 5_000;

pub struct NativeWorkspace {
    external_scopes: std::sync::Arc<std::sync::Mutex<external_scopes::Registry>>,
    log_store: Option<Result<disk_logs::DiskLogs, Failure>>,
    project: Project,
    source_queries: workspace::services::SourceQueryService,
    index_maintenance: workspace::services::IndexMaintenanceService,
    source_editing: workspace::services::SourceEditingService,
    session_service: workspace::services::SessionService,
    browser: std::sync::Arc<dyn harness_application::ports::browser::BrowserInspection>,
    tasks: Option<tasks::TaskManager>,
    index_watch: Option<std::sync::Arc<std::sync::Mutex<Value>>>,
}
pub type NativeApplication = Application<workspace_set::MultiNativeWorkspace, sessions::Sessions>;

pub(crate) fn workspace_dependencies(
    root: &Path,
    options: NativeApplicationOptions,
) -> std::io::Result<(NativeWorkspace, sessions::Sessions)> {
    composition::dependencies(root, options)
}

/// Optional infrastructure supplied by the native composition root.
/// Defaults preserve the lightweight constructors used by library callers.
#[derive(Default)]
pub(crate) struct NativeApplicationOptions {
    pub(crate) browser:
        Option<std::sync::Arc<dyn harness_application::ports::browser::BrowserInspection>>,
    pub(crate) persist_logs: bool,
    pub(crate) tasks: Option<tasks::TaskManager>,
    pub(crate) index_watch: Option<std::sync::Arc<std::sync::Mutex<Value>>>,
}

pub fn application(root: &Path) -> std::io::Result<NativeApplication> {
    application_with_options(root, NativeApplicationOptions::default())
}
pub fn application_with_logging(
    root: &Path,
    persist_logs: bool,
) -> std::io::Result<NativeApplication> {
    application_with_options(
        root,
        NativeApplicationOptions {
            persist_logs,
            ..NativeApplicationOptions::default()
        },
    )
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
    application_with_options(
        root,
        NativeApplicationOptions {
            persist_logs,
            tasks,
            index_watch,
            ..NativeApplicationOptions::default()
        },
    )
}

pub(crate) fn application_with_options(
    root: &Path,
    options: NativeApplicationOptions,
) -> std::io::Result<NativeApplication> {
    let (workspace, sessions) = composition::dependencies(root, options)?;
    let project = workspace_set::WorkspaceProject {
        name: "default".to_owned(),
        root: workspace.project.root.clone(),
    };
    Ok(Application::new(
        workspace_set::MultiNativeWorkspace {
            projects: vec![(project, std::sync::Arc::new(workspace))],
            workspace_mode: false,
            anchor: None,
            reload_status: None,
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
        workspace::services::read_scoped_project(
            &self.project,
            &self.external_scopes,
            scope,
            session,
            context,
            read,
        )
    }
}
