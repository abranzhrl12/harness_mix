use harness_domain::{
    Failure,
    contracts::{ReadArgs, SearchArgs},
    execution::OperationContext,
};
use serde_json::Value;

pub mod parsing;

/// Platform-dependent I/O. Implementations enforce filesystem safety and scan budgets.
pub trait Workspace: Send + Sync {
    fn explore(
        &self,
        _args: &harness_domain::contracts::ExploreArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Graph exploration adapter unavailable"))
    }
    fn external_scope(
        &self,
        _args: &harness_domain::contracts::ExternalScopeArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "External scope adapter unavailable"))
    }
    fn config(
        &self,
        _args: &harness_domain::contracts::ConfigArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Configuration section adapter unavailable"))
    }
    fn log_persistence_enabled(&self) -> bool {
        false
    }
    fn load_log_archive(&self) -> Result<Option<Value>, Failure> {
        Ok(None)
    }
    fn persist_log_archive(&self, _archive: &Value) -> Result<u64, Failure> {
        Err(("UNSUPPORTED", "Log persistence unavailable"))
    }
    fn doctor(
        &self,
        _args: &harness_domain::contracts::DoctorArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Doctor adapter unavailable"))
    }
    fn git_status(
        &self,
        _args: &harness_domain::contracts::GitStatusArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Git status adapter unavailable"))
    }
    fn git_session_changes(
        &self,
        _edits: &[Value],
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Ok(serde_json::json!([]))
    }
    fn git_diff(
        &self,
        _args: &harness_domain::contracts::GitDiffArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Git diff adapter unavailable"))
    }
    fn sql_status(
        &self,
        _args: &harness_domain::sql::SqlStatusArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "SQL status adapter unavailable"))
    }
    fn sql_search(
        &self,
        _args: &harness_domain::sql::SqlSearchArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "SQL search adapter unavailable"))
    }
    fn sql_describe(
        &self,
        _args: &harness_domain::sql::SqlDescribeArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "SQL describe adapter unavailable"))
    }
    fn sql_query(
        &self,
        _args: &harness_domain::sql::SqlQueryArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "SQL query adapter unavailable"))
    }
    fn xlsx_inspect(
        &self,
        _args: &harness_domain::xlsx::XlsxInspectArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "XLSX inspect adapter unavailable"))
    }
    fn xlsx_read(
        &self,
        _args: &harness_domain::xlsx::XlsxReadArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "XLSX read adapter unavailable"))
    }
    fn svg(
        &self,
        _args: &harness_domain::svg::SvgArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "SVG adapter unavailable"))
    }
    fn handover(
        &self,
        _args: &harness_domain::contracts::HandoverArgs,
        _snapshot: &Value,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Handover adapter unavailable"))
    }
    fn process_status(
        &self,
        _args: &harness_domain::contracts::ProcessStatusArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Process status adapter unavailable"))
    }
    fn diagnostics(
        &self,
        _args: &harness_domain::contracts::DiagnosticsArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Diagnostics adapter unavailable"))
    }
    fn data_query(
        &self,
        _args: &harness_domain::contracts::DataQueryArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Data query adapter unavailable"))
    }
    /// Bounded filesystem observation; called outside the session repository mutex.
    fn observe_workspace(&self, _context: &OperationContext) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Workspace observation adapter unavailable"))
    }
    fn persist_session(
        &self,
        _id: &str,
        _snapshot: &Value,
        _context: &OperationContext,
    ) -> Result<(), Failure> {
        Err(("PERSISTENCE", "Session persistence adapter unavailable"))
    }
    fn outline(
        &self,
        _args: &harness_domain::contracts::OutlineArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Outline adapter unavailable"))
    }
    fn read_symbol(
        &self,
        _args: &harness_domain::contracts::ReadSymbolArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Symbol adapter unavailable"))
    }
    fn vue_extract_preview(
        &self,
        _args: &harness_domain::contracts::VueExtractPreviewArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Vue extraction preview adapter unavailable"))
    }
    fn patch_preview(
        &self,
        _args: &harness_domain::contracts::ReplaceArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Patch preview adapter unavailable"))
    }
    fn replace(
        &self,
        _args: &harness_domain::contracts::ReplaceArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Editing adapter unavailable"))
    }
    fn move_file(
        &self,
        _args: &harness_domain::contracts::MoveArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Move adapter unavailable"))
    }
    fn write(
        &self,
        _args: &harness_domain::contracts::WriteArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "File creation adapter unavailable"))
    }
    fn info(&self) -> Value;
    fn rules(&self, context: &OperationContext) -> Result<String, Failure>;
    fn capabilities(&self) -> Value;
    fn read(&self, args: &ReadArgs, context: &OperationContext) -> Result<Value, Failure>;
    fn search(&self, args: &SearchArgs, context: &OperationContext) -> Result<Value, Failure>;
    fn ignore(
        &self,
        _args: &harness_domain::contracts::IgnoreArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Ignore policy adapter unavailable"))
    }
    fn verify(
        &self,
        _args: &harness_domain::contracts::VerifyArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Verification adapter unavailable"))
    }
}

/// Internal execution only; callers validate tool-specific arguments and scope.
/// Implementations drain both pipes concurrently and enforce a shared byte budget.
/// Admission must remain held until the child has been reaped.
pub trait ProcessRunner: Send + Sync {
    fn run(
        &self,
        spec: &harness_domain::process::ProcessSpec,
        context: &OperationContext,
    ) -> Result<harness_domain::process::ProcessOutcome, Failure>;
}

pub struct SessionContext {
    pub id: String,
    pub restored: bool,
    pub rules_hash: Option<String>,
    pub capabilities_hash: Option<String>,
    pub task_summary: Value,
    pub persistence_warning: Option<Value>,
}

/// The application decides when to open/cache a session; the adapter owns storage.
pub trait SessionRepository: Send {
    /// Return the same gate for a pinned session throughout its cache lifetime.
    fn workflow_gate(&mut self, id: &str) -> Result<std::sync::Arc<std::sync::Mutex<()>>, Failure>;
    fn tasks(
        &mut self,
        _args: &harness_domain::contracts::TasksArgs,
    ) -> Result<SessionMutation, Failure> {
        Err(("UNSUPPORTED", "Tasks adapter unavailable"))
    }
    fn checkpoint(
        &mut self,
        _args: &harness_domain::contracts::CheckpointArgs,
    ) -> Result<SessionMutation, Failure> {
        Err(("UNSUPPORTED", "Checkpoint adapter unavailable"))
    }
    fn snapshot(&mut self, _id: &str) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Session snapshot adapter unavailable"))
    }
    /// Retain negative freshness facts in live checks; never revive historical checks.
    fn invalidate_checks(&mut self, _id: &str, _observation: &Value) -> Result<(), Failure> {
        Ok(())
    }
    fn record(
        &mut self,
        _id: &str,
        _tool: &str,
        _args: &Value,
        _result: &Value,
    ) -> Result<Option<Value>, Failure> {
        Ok(None)
    }
    fn resolve_id(&mut self, id: &str) -> Result<String, Failure> {
        Ok(id.to_string())
    }
    fn acquire(&mut self, id: &str) -> Result<(), Failure>;
    fn release(&mut self, id: &str);
    fn open(&mut self, id: Option<&str>) -> Result<SessionContext, Failure>;
    fn update_hashes(
        &mut self,
        id: &str,
        rules: String,
        capabilities: String,
    ) -> Result<(), Failure>;
}

/// Owned snapshot lets the workspace persist outside the global repository mutex.
pub struct SessionMutation {
    pub data: Value,
    pub snapshot: Option<Value>,
}
