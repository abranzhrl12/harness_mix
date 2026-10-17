//! Native implementations of application ports. No MCP handlers or schemas here.
mod capabilities;
mod disk_logs;
mod doctor;
mod edit;
mod edit_commit;
mod edit_security;
mod fingerprint;
mod git;
pub mod process;
mod process_tree;
mod project;
mod rust_symbols;
mod rust_syntax;
mod search;
mod session_workflow;
mod sessions;
mod symbols;
mod verify;

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
    log_store: Option<Result<disk_logs::DiskLogs, Failure>>,
    project: Project,
    edit_lock: std::sync::Mutex<()>,
}
pub type NativeApplication = Application<NativeWorkspace, sessions::Sessions>;

pub fn application(root: &Path) -> std::io::Result<NativeApplication> {
    application_with_logging(root, false)
}
pub fn application_with_logging(
    root: &Path,
    persist_logs: bool,
) -> std::io::Result<NativeApplication> {
    let project = Project::new(root)?;
    let sessions = sessions::Sessions::new(project.clone());
    let log_store = persist_logs.then(|| disk_logs::DiskLogs::new(project.clone()));
    Ok(Application::new(
        NativeWorkspace {
            log_store,
            project,
            edit_lock: std::sync::Mutex::new(()),
        },
        sessions,
    ))
}

impl Workspace for NativeWorkspace {
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
        doctor::doctor(&self.project, args, context)
    }
    fn git_status(
        &self,
        args: &harness_domain::contracts::GitStatusArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        git::status(&self.project, args, context)
    }
    fn git_diff(
        &self,
        args: &harness_domain::contracts::GitDiffArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        git::diff(&self.project, args, context)
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
        symbols::outline(&self.project, args, context)
    }
    fn read_symbol(
        &self,
        args: &harness_domain::contracts::ReadSymbolArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        symbols::read_symbol(&self.project, args, context)
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
        json!({"root":root,"languages":languages,"manifestScope":"raíz del proyecto","indexed":self.project.root.join(".codegraph").is_dir()})
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
        self.project
            .read_with_context(&args.path, args.start, args.end, context)
    }
    fn search(&self, args: &SearchArgs, context: &OperationContext) -> Result<Value, Failure> {
        search::search_with_context(&self.project, args, context)
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
    fn record(
        &mut self,
        id: &str,
        tool: &str,
        args: &Value,
        result: &Value,
    ) -> Result<Option<Value>, Failure> {
        self.record_observed(id, tool, args, result)
    }
    fn acquire(&mut self, id: &str) -> Result<(), Failure> {
        self.acquire(id)
    }
    fn release(&mut self, id: &str) {
        self.release(id);
    }
    fn open(&mut self, id: Option<&str>) -> Result<SessionContext, Failure> {
        let (id, warning) = match id {
            Some(id) => (id.to_owned(), None),
            None => self.create()?,
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
