use super::{
    BatchArgs, BootstrapArgs, CheckpointArgs, ConfigArgs, DataQueryArgs, DeleteArgs, DetailArgs,
    DiagnosticsArgs, DoctorArgs, ExploreArgs, ExternalScopeAction, ExternalScopeArgs, GitDiffArgs,
    GitStatusArgs, HandoverArgs, IgnoreAction, IgnoreArgs, LogsArgs, MoveArgs, OutlineArgs,
    ProcessStatusArgs, ProposalArgs, ReadArgs, ReadSymbolArgs, ReplaceArgs, SearchArgs,
    TasksAction, TasksArgs, VerifyArgs, VueExtractPreviewArgs, WriteArgs,
};
use crate::browser::BrowserArgs;
use serde_json::Value;

pub enum Request {
    Browser(BrowserArgs),
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
        if let Self::Verify(args) = self {
            // The request deadline includes bounded process-tree cleanup after the requested process limit.
            args.effective_timeout()
                .unwrap_or_else(|_| {
                    std::time::Duration::from_secs(
                        crate::contracts::verification::DEFAULT_VERIFICATION_TIMEOUT_SECONDS,
                    )
                })
                .saturating_add(std::time::Duration::from_secs(5))
        } else {
            crate::execution::DEFAULT_TIMEOUT
        }
    }
    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            Self::Browser(args) if args.is_mutating()
        ) || matches!(
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
            Self::Browser(_) => "harness_browser",
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
            Self::Browser(args) => serde_json::to_value(args),
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

#[cfg(test)]
mod browser_request_tests {
    use super::*;

    #[test]
    fn browser_request_keeps_tool_name_and_serialized_action() {
        let request = Request::Browser(BrowserArgs::Connect {
            endpoint: "http://127.0.0.1:9222".into(),
            tab: None,
            session: Some("session-1".into()),
        });
        assert_eq!(request.name(), "harness_browser");
        assert_eq!(request.arguments()["action"], "connect");
        assert_eq!(request.arguments()["session"], "session-1");
        assert!(request.is_mutating());
    }

    #[test]
    fn browser_connection_lifecycle_is_mutating_but_inspection_is_read_only() {
        let read = Request::Browser(BrowserArgs::Inspect {
            connection_id: "connection-1".into(),
            target_id: "target-1".into(),
            selector: None,
            max_chars: Some(1024),
            session: Some("session-1".into()),
        });
        let attach = Request::Browser(BrowserArgs::Attach {
            connection_id: "connection-1".into(),
            target_id: "target-1".into(),
            session: Some("session-1".into()),
        });
        assert!(!read.is_mutating());
        assert!(attach.is_mutating());
    }
}

#[cfg(test)]
mod verify_timeout_tests {
    use super::*;

    fn args(timeout_seconds: Option<u64>) -> VerifyArgs {
        VerifyArgs {
            language: "rust".into(),
            scope: None,
            package: None,
            project: None,
            configuration: None,
            action: None,
            engine: None,
            warnings: None,
            timeout_seconds,
            session: None,
        }
    }

    #[test]
    fn verify_timeout_defaults_and_adds_cleanup_grace() {
        assert_eq!(
            args(None).effective_timeout().unwrap(),
            std::time::Duration::from_secs(
                crate::contracts::verification::DEFAULT_VERIFICATION_TIMEOUT_SECONDS
            )
        );
        assert_eq!(
            Request::Verify(args(None)).execution_timeout(),
            std::time::Duration::from_secs(905)
        );
        assert_eq!(
            Request::Verify(args(Some(3600))).execution_timeout(),
            std::time::Duration::from_secs(3605)
        );
    }

    #[test]
    fn invalid_programmatic_timeout_returns_failure_and_request_uses_safe_default() {
        let invalid = args(Some(0));
        assert_eq!(invalid.effective_timeout().unwrap_err().0, "VERIFY_TIMEOUT");
        assert_eq!(
            Request::Verify(invalid).execution_timeout(),
            std::time::Duration::from_secs(905)
        );
    }

    #[test]
    fn verify_timeout_rejects_zero_and_values_above_maximum() {
        for seconds in [0, 3601] {
            let parsed = serde_json::from_value::<VerifyArgs>(serde_json::json!({
                "language": "rust",
                "timeoutSeconds": seconds
            }));
            assert!(parsed.is_err(), "timeoutSeconds={seconds} must be invalid");
        }
    }
}
