//! Native implementations of application ports. No MCP handlers or schemas here.
mod index_razor_mvc;
mod parsers;
mod vue_extract;
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
pub(crate) mod doctor;
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
    ports::{SessionContext, SessionRepository, Workspace},
};
use harness_domain::{
    Failure,
    contracts::{ReadArgs, SearchArgs},
    execution::OperationContext,
};
use project::Project;
use serde_json::{Value, json};
use std::path::Path;

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

impl Workspace for NativeWorkspace {
    fn explore(
        &self,
        args: &harness_domain::contracts::ExploreArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        if args.sync_index == Some(true) {
            let _guard = self
                .edit_lock
                .lock()
                .map_err(|_| ("INTERNAL", "Workspace mutation lock unavailable"))?;
            context.check()?;
            explore::synchronized(&self.project, args, context)
        } else {
            explore::explore(&self.project, args, context)
        }
    }
    fn external_scope(
        &self,
        args: &harness_domain::contracts::ExternalScopeArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        use harness_domain::contracts::ExternalScopeAction;
        context.check()?;
        match args.action {
            ExternalScopeAction::Grant => {
                if args.user_authorized != Some(true) {
                    return Err((
                        "AUTHORIZATION",
                        "Grant requires explicit user consent for this exact path and task",
                    ));
                }
                let approved = external_scopes::approve(
                    args.root
                        .as_deref()
                        .ok_or(("ARGUMENT", "grant requires root"))?,
                    context,
                )?;
                self.external_scopes
                    .lock()
                    .map_err(|_| ("INTERNAL", "External registry unavailable"))?
                    .grant(&args.session, approved, context)
            }
            ExternalScopeAction::List => Ok(self
                .external_scopes
                .lock()
                .map_err(|_| ("INTERNAL", "External registry unavailable"))?
                .list(&args.session)),
            ExternalScopeAction::Revoke => self
                .external_scopes
                .lock()
                .map_err(|_| ("INTERNAL", "External registry unavailable"))?
                .revoke(
                    &args.session,
                    args.scope_id
                        .as_deref()
                        .ok_or(("ARGUMENT", "revoke requires scopeId"))?,
                ),
        }
    }
    fn config(
        &self,
        args: &harness_domain::contracts::ConfigArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        if args.external_scope.is_some() {
            return self.read_project(
                args.external_scope.as_deref(),
                args.session.as_deref(),
                context,
                |project, _| config::section(project, args, context),
            );
        }
        config::section(&self.project, args, context)
    }
    fn log_persistence_enabled(&self) -> bool {
        self.log_store.is_some()
    }
    fn load_log_archive(&self) -> Result<Option<Value>, Failure> {
        match &self.log_store {
            Some(Ok(store)) => store.load(),
            Some(Err(error)) => Err(*error),
            None => Ok(None),
        }
    }
    fn persist_log_archive(&self, archive: &Value) -> Result<u64, Failure> {
        match &self.log_store {
            Some(Ok(store)) => store.save(archive),
            Some(Err(error)) => Err(*error),
            None => Err(("UNSUPPORTED", "Disk logging disabled")),
        }
    }
    fn doctor(
        &self,
        args: &harness_domain::contracts::DoctorArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let mut result = doctor::doctor(&self.project, args, context)?;
        let details = self
            .index_watch
            .as_ref()
            .and_then(|status| status.lock().ok().map(|v| v.clone()))
            .unwrap_or_else(|| json!({"enabled":false,"state":"disabled"}));
        doctor::append_check(
            &mut result,
            json!({
                "id":"index-auto-sync",
                "status":if !details["lastError"].is_null() { "warning" } else if details["enabled"] != true { "skipped" } else { "ok" },
                "message":"Automatic index maintenance status; global freshness not certified",
                "details":details
            }),
        );
        match &self.tasks {
            Some(tasks) => {
                let diagnostics = tasks.diagnostics();
                doctor::append_check(
                    &mut result,
                    json!({
                        "id":"background-jobs", "status":diagnostics["status"],
                        "message":"Shared job registry and existing durable history inspected; no storage created or write probe performed",
                        "details":diagnostics
                    }),
                );
            }
            None => doctor::append_check(
                &mut result,
                json!({
                    "id":"background-jobs", "status":"skipped",
                    "message":"Background job manager not attached in this application/profile"
                }),
            ),
        }
        Ok(result)
    }
    fn git_status(
        &self,
        args: &harness_domain::contracts::GitStatusArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        git::status(&self.project, args, context)
    }
    fn git_session_changes(
        &self,
        edits: &[Value],
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        git::session_changes(&self.project, edits, context)
    }
    fn git_diff(
        &self,
        args: &harness_domain::contracts::GitDiffArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        git::diff(&self.project, args, context)
    }
    fn sql_status(
        &self,
        args: &harness_domain::sql::SqlStatusArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        sql::status(&self.project, args, context)
    }
    fn sql_search(
        &self,
        args: &harness_domain::sql::SqlSearchArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        sql::search(&self.project, args, context)
    }
    fn sql_describe(
        &self,
        args: &harness_domain::sql::SqlDescribeArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        sql::describe(&self.project, args, context)
    }
    fn sql_query(
        &self,
        args: &harness_domain::sql::SqlQueryArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        sql::query(&self.project, args, context)
    }
    fn xlsx_inspect(
        &self,
        args: &harness_domain::xlsx::XlsxInspectArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.read_project(
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, _| xlsx::inspect(project, args, context),
        )
    }
    fn xlsx_read(
        &self,
        args: &harness_domain::xlsx::XlsxReadArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.read_project(
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, _| xlsx::read(project, args, context),
        )
    }
    fn svg(
        &self,
        args: &harness_domain::svg::SvgArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.read_project(
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, _| svg::handle_svg(project, args, context),
        )
    }
    fn handover(
        &self,
        args: &harness_domain::contracts::HandoverArgs,
        snapshot: &Value,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        handover::handle_handover(&self.project, args, snapshot, context)
    }
    fn process_status(
        &self,
        args: &harness_domain::contracts::ProcessStatusArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        process_status::handle_process_status_with_tasks(args, context, self.tasks.as_ref())
    }
    fn diagnostics(
        &self,
        args: &harness_domain::contracts::DiagnosticsArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        diagnostics::handle_diagnostics(&self.project, args, context)
    }
    fn data_query(
        &self,
        args: &harness_domain::contracts::DataQueryArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        data_query::handle_data_query(&self.project, args, context)
    }
    fn observe_workspace(&self, context: &OperationContext) -> Result<Value, Failure> {
        let captured = fingerprint::capture(&self.project, context);
        context.check()?;
        let time = sessions::now();
        Ok(match captured {
            Ok(hash) => json!({"status":"captured","hash":hash,"capturedAt":time,
                "coverage":fingerprint::coverage()}),
            Err((code, _)) => json!({"status":"unknown","error":code,"capturedAt":time,
                "coverage":fingerprint::coverage()}),
        })
    }
    fn persist_session(
        &self,
        id: &str,
        snapshot: &Value,
        context: &OperationContext,
    ) -> Result<(), Failure> {
        sessions::persist_snapshot(&self.project, id, snapshot, context)
    }
    fn outline(
        &self,
        args: &harness_domain::contracts::OutlineArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.read_project(
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, _| symbols::outline(project, args, context),
        )
    }
    fn read_symbol(
        &self,
        args: &harness_domain::contracts::ReadSymbolArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.read_project(
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, _| symbols::read_symbol(project, args, context),
        )
    }
    fn vue_extract_preview(
        &self,
        args: &harness_domain::contracts::VueExtractPreviewArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        vue_extract::extract_preview(&self.project, args, context)
    }
    fn patch_preview(
        &self,
        args: &harness_domain::contracts::ReplaceArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        edit::preview(&self.project, args, context)
    }
    fn replace(
        &self,
        args: &harness_domain::contracts::ReplaceArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let _lock = self.edit_lock.try_lock().map_err(|_| {
            (
                "EDIT_BUSY",
                "Another edit is active; read again before retrying",
            )
        })?;
        edit::replace(&self.project, args, context)
    }
    fn write(
        &self,
        args: &harness_domain::contracts::WriteArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let _lock = self
            .edit_lock
            .try_lock()
            .map_err(|_| ("EDIT_BUSY", "Another edit is active"))?;
        edit::write(&self.project, args, context)
    }
    fn move_file(
        &self,
        args: &harness_domain::contracts::MoveArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let _lock = self.edit_lock.try_lock().map_err(|_| {
            (
                "EDIT_BUSY",
                "Another edit or move is active; retry after it completes",
            )
        })?;
        move_file::move_file(&self.project, args, context)
    }
    fn info(&self) -> Value {
        let root = self.project.root.to_string_lossy();
        let root = root.strip_prefix(r"\\?\").unwrap_or(&root);
        let manifests = [
            ("rust", "Cargo.toml"),
            ("go", "go.mod"),
            ("javascript", "package.json"),
            ("python", "pyproject.toml"),
            ("java", "pom.xml"),
            ("dotnet", "global.json"),
        ];
        let mut languages: Vec<&str> = manifests
            .iter()
            .filter(|(_, file)| self.project.root.join(file).is_file())
            .map(|(language, _)| *language)
            .collect();
        if !languages.contains(&"python") && self.project.root.join("requirements.txt").is_file() {
            languages.push("python");
        }
        if !languages.contains(&"dotnet")
            && std::fs::read_dir(&self.project.root)
                .ok()
                .is_some_and(|items| {
                    items.filter_map(Result::ok).any(|entry| {
                        entry.path().extension().is_some_and(|ext| {
                            ["csproj", "sln", "slnx"]
                                .iter()
                                .any(|s| ext.eq_ignore_ascii_case(s))
                        })
                    })
                })
        {
            languages.push("dotnet");
        }
        json!({"root":root,"languages":languages,"implementationVersion":env!("CARGO_PKG_VERSION"),"manifestScope":"raíz del proyecto","indexed":self.project.root.join(".codegraph").is_dir()})
    }
    fn rules(&self, context: &OperationContext) -> Result<String, Failure> {
        let relative = ".agents/rules/00-mandatory-tools.md";
        if !self.project.root.join(relative).exists() {
            return Ok(String::new());
        }
        String::from_utf8(self.project.bytes_with_context(relative, context)?)
            .map_err(|_| ("ENCODING", "reglas no UTF-8"))
    }
    fn capabilities(&self) -> Value {
        capabilities::capabilities()
    }
    fn read(&self, args: &ReadArgs, context: &OperationContext) -> Result<Value, Failure> {
        self.read_project(
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, _| project.read_with_context(&args.path, args.start, args.end, context),
        )
    }
    fn search(&self, args: &SearchArgs, context: &OperationContext) -> Result<Value, Failure> {
        self.read_project(
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, default_scope| {
                if args.scope.is_none() && default_scope.is_some() {
                    let mut value = serde_json::to_value(args)
                        .map_err(|_| ("INTERNAL", "Cannot encode search arguments"))?;
                    value["scope"] = json!(default_scope);
                    let scoped = serde_json::from_value(value)
                        .map_err(|_| ("INTERNAL", "Cannot scope search arguments"))?;
                    search::search_with_context(project, &scoped, context)
                } else {
                    search::search_with_context(project, args, context)
                }
            },
        )
    }
    fn ignore(
        &self,
        args: &harness_domain::contracts::IgnoreArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        ignore::ignore(&self.project, args, context)
    }
    fn verify(
        &self,
        args: &harness_domain::contracts::VerifyArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        verify::verify(&self.project, args, context)
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
