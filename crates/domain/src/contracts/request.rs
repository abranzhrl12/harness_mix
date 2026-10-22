use super::{
    BatchArgs, BootstrapArgs, CheckpointArgs, ConfigArgs, DataQueryArgs, DeleteArgs, DetailArgs,
    DiagnosticsArgs, DoctorArgs, ExploreArgs, ExternalScopeAction, ExternalScopeArgs, GitDiffArgs,
    GitStatusArgs, HandoverArgs, IgnoreAction, IgnoreArgs, LogsArgs, MoveArgs, OutlineArgs,
    ProcessStatusArgs, ProposalArgs, ReadArgs, ReadSymbolArgs, ReplaceArgs, SearchArgs,
    TasksAction, TasksArgs, VerifyArgs, VueExtractPreviewArgs, WriteArgs,
};
use serde_json::Value;

pub enum Request {
    PatchPreview(ReplaceArgs),
    Explore(ExploreArgs),
    ExternalScope(ExternalScopeArgs),
    Config(ConfigArgs),
    Logs(LogsArgs),
    Batch(BatchArgs),
    Doctor(DoctorArgs),
    Bootstrap(BootstrapArgs),
    Read(ReadArgs),
    Detail(DetailArgs),
    Search(SearchArgs),
    Verify(VerifyArgs),
    Replace(ReplaceArgs),
    Write(WriteArgs),
    Move(MoveArgs),
    Delete(DeleteArgs),
    Proposal(ProposalArgs),
    Outline(OutlineArgs),
    ReadSymbol(ReadSymbolArgs),
    VueExtractPreview(VueExtractPreviewArgs),
    Tasks(TasksArgs),
    Checkpoint(CheckpointArgs),
    GitStatus(GitStatusArgs),
    GitDiff(GitDiffArgs),
    SqlStatus(crate::sql::SqlStatusArgs),
    SqlSearch(crate::sql::SqlSearchArgs),
    SqlMap(crate::sql::SqlMapArgs),
    SqlDescribe(crate::sql::SqlDescribeArgs),
    SqlQuery(crate::sql::SqlQueryArgs),
    XlsxInspect(crate::xlsx::XlsxInspectArgs),
    XlsxRead(crate::xlsx::XlsxReadArgs),
    Svg(crate::svg::SvgArgs),
    Handover(HandoverArgs),
    ProcessStatus(ProcessStatusArgs),
    Diagnostics(DiagnosticsArgs),
    DataQuery(DataQueryArgs),
    Ignore(IgnoreArgs),
}
impl Request {
    pub fn execution_timeout(&self) -> std::time::Duration {
        if matches!(self, Self::Verify(_)) {
            // Process deadline plus a bounded window to collect output and stop its tree.
            std::time::Duration::from_secs(65)
        } else {
            crate::execution::DEFAULT_TIMEOUT
        }
    }
    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            Self::Replace(_)
                | Self::Write(_)
                | Self::Move(_)
                | Self::Delete(_)
                | Self::Proposal(_)
                | Self::ExternalScope(ExternalScopeArgs {
                    action: ExternalScopeAction::Grant | ExternalScopeAction::Revoke,
                    ..
                })
                | Self::Tasks(TasksArgs {
                    action: Some(TasksAction::Update),
                    ..
                })
        ) || matches!(self, Self::Checkpoint(args) if args.has_updates())
            || matches!(self, Self::Handover(args) if args.target_file.is_some())
            || matches!(self, Self::Explore(args) if args.sync_index == Some(true))
            || matches!(self, Self::Ignore(args) if args.action != IgnoreAction::List)
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::PatchPreview(_) => "harness_patch_preview",
            Self::Explore(_) => "harness_explore",
            Self::ExternalScope(_) => "harness_external_scope",
            Self::Config(_) => "harness_config",
            Self::Logs(_) => "harness_doctor",
            Self::Batch(_) => "harness_batch",
            Self::Doctor(_) => "harness_doctor",
            Self::Bootstrap(_) => "harness_bootstrap",
            Self::Read(_) => "harness_read",
            Self::Detail(_) => "harness_detail",
            Self::Search(_) => "harness_search",
            Self::Verify(_) => "harness_verify",
            Self::Replace(_) => "harness_replace",
            Self::Write(_) => "harness_write",
            Self::Move(_) => "harness_move",
            Self::Delete(_) => "harness_delete",
            Self::Proposal(_) => "harness_proposal",
            Self::Outline(_) | Self::ReadSymbol(_) | Self::VueExtractPreview(_) => {
                "harness_symbols"
            }
            Self::Tasks(_) => "harness_tasks",
            Self::Checkpoint(_) => "harness_checkpoint",
            Self::GitStatus(_) | Self::GitDiff(_) => "harness_git",
            Self::SqlStatus(_)
            | Self::SqlSearch(_)
            | Self::SqlMap(_)
            | Self::SqlDescribe(_)
            | Self::SqlQuery(_) => "harness_sql",
            Self::XlsxInspect(_) | Self::XlsxRead(_) => "harness_xlsx",
            Self::Svg(_) => "harness_svg",
            Self::Handover(_) => "harness_handover",
            Self::ProcessStatus(_) | Self::Diagnostics(_) => "harness_doctor",
            Self::DataQuery(_) => "harness_data_query",
            Self::Ignore(_) => "harness_ignore",
        }
    }
    pub fn arguments(&self) -> Value {
        match self {
            Self::PatchPreview(args) => serde_json::to_value(args),
            Self::Explore(args) => serde_json::to_value(args),
            Self::ExternalScope(args) => serde_json::to_value(args),
            Self::Config(args) => serde_json::to_value(args),
            Self::Logs(args) => serde_json::to_value(args),
            Self::Batch(args) => serde_json::to_value(args),
            Self::Doctor(args) => serde_json::to_value(args),
            Self::Bootstrap(args) => serde_json::to_value(args),
            Self::Read(args) => serde_json::to_value(args),
            Self::Detail(args) => serde_json::to_value(args),
            Self::Search(args) => serde_json::to_value(args),
            Self::Verify(args) => serde_json::to_value(args),
            Self::Replace(args) => serde_json::to_value(args),
            Self::Write(args) => serde_json::to_value(args),
            Self::Move(args) => serde_json::to_value(args),
            Self::Delete(args) => serde_json::to_value(args),
            Self::Proposal(args) => serde_json::to_value(args),
            Self::Outline(args) => serde_json::to_value(args).map(|mut value| {
                value["action"] = Value::String("outline".into());
                value
            }),
            Self::ReadSymbol(args) => serde_json::to_value(args).map(|mut value| {
                value["action"] = Value::String("read".into());
                value
            }),
            Self::VueExtractPreview(args) => serde_json::to_value(args).map(|mut value| {
                value["action"] = Value::String("extract_preview".into());
                value
            }),
            Self::Tasks(args) => serde_json::to_value(args),
            Self::Checkpoint(args) => serde_json::to_value(args),
            Self::GitStatus(args) => serde_json::to_value(args).map(|mut value| {
                value["action"] = Value::String("status".into());
                value
            }),
            Self::GitDiff(args) => serde_json::to_value(args).map(|mut value| {
                value["action"] = Value::String("diff".into());
                value
            }),
            Self::SqlStatus(args) => serde_json::to_value(args),
            Self::SqlSearch(args) => serde_json::to_value(args),
            Self::SqlMap(args) => serde_json::to_value(args),
            Self::SqlDescribe(args) => serde_json::to_value(args),
            Self::SqlQuery(args) => serde_json::to_value(args),
            Self::XlsxInspect(args) => serde_json::to_value(args),
            Self::XlsxRead(args) => serde_json::to_value(args),
            Self::Svg(args) => serde_json::to_value(args),
            Self::Handover(args) => serde_json::to_value(args),
            Self::ProcessStatus(args) => serde_json::to_value(args),
            Self::Diagnostics(args) => serde_json::to_value(args),
            Self::DataQuery(args) => serde_json::to_value(args),
            Self::Ignore(args) => serde_json::to_value(args),
        }
        .expect("request arguments serialize")
    }
}
