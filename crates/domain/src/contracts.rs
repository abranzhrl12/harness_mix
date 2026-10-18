use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct BootstrapArgs {
    /// Reuse data.session from bootstrap. sessionId is accepted for compatibility.
    #[serde(alias = "sessionId")]
    pub session: Option<String>,
    #[serde(rename = "includeRules")]
    pub include_rules: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ReadArgs {
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    pub path: String,
    pub start: usize,
    pub end: usize,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DetailArgs {
    #[serde(rename = "ref")]
    pub reference: String,
    pub offset: Option<usize>,
    #[serde(rename = "maxChars")]
    pub max_chars: Option<usize>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Text,
    Files,
    FileSize,
    Syntax,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SearchArgs {
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    pub kind: Kind,
    #[serde(default)]
    pub query: String,
    #[serde(rename = "minLines")]
    pub min_lines: Option<usize>,
    pub scope: Option<String>,
    pub limit: Option<usize>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VerifyAction {
    Build,
    Test,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct VerifyArgs {
    pub language: String,
    pub scope: Option<String>,
    pub action: Option<VerifyAction>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[serde(try_from = "ReplaceInput")]
pub struct ReplaceArgs {
    pub path: String,
    pub hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunks: Option<Vec<ReplaceChunk>>,
    #[serde(default, rename = "oldText")]
    pub old_text: String,
    #[serde(default, rename = "newText")]
    pub new_text: String,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MoveArgs {
    pub from: String,
    pub to: String,
    pub session: Option<String>,
}

impl Serialize for ReplaceArgs {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("path", &self.path)?;
        map.serialize_entry("hash", &self.hash)?;
        if let Some(chunks) = &self.chunks {
            map.serialize_entry("chunks", chunks)?;
        } else {
            map.serialize_entry("oldText", &self.old_text)?;
            map.serialize_entry("newText", &self.new_text)?;
        }
        map.serialize_entry("session", &self.session)?;
        map.end()
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ReplaceChunk {
    #[serde(rename = "oldText")]
    pub old_text: String,
    #[serde(rename = "newText")]
    pub new_text: String,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct ReplaceInput {
    path: String,
    hash: String,
    chunks: Option<Vec<ReplaceChunk>>,
    #[serde(rename = "oldText")]
    old_text: Option<String>,
    #[serde(rename = "newText")]
    new_text: Option<String>,
    session: Option<String>,
}

impl TryFrom<ReplaceInput> for ReplaceArgs {
    type Error = &'static str;

    fn try_from(input: ReplaceInput) -> Result<Self, Self::Error> {
        let (old_text, new_text) = match (&input.chunks, input.old_text, input.new_text) {
            (Some(_), None, None) => (String::new(), String::new()),
            (None, Some(old), Some(new)) => (old, new),
            _ => return Err("Use either chunks or both oldText and newText"),
        };
        Ok(Self {
            path: input.path,
            hash: input.hash,
            chunks: input.chunks,
            old_text,
            new_text,
            session: input.session,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct OutlineArgs {
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    pub path: String,
    pub query: Option<String>,
    #[serde(rename = "maxChars")]
    pub max_chars: Option<usize>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SymbolView {
    Full,
    Signature,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ReadSymbolArgs {
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    pub path: String,
    pub symbol: String,
    pub container: Option<String>,
    #[serde(rename = "symbolId")]
    pub symbol_id: Option<String>,
    pub view: Option<SymbolView>,
    /// Copy data.session from bootstrap.
    #[serde(alias = "sessionId")]
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum TasksAction {
    Status,
    Update,
    Timeline,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TasksArgs {
    pub session: String,
    pub action: Option<TasksAction>,
    pub phase: Option<String>,
    pub status: Option<String>,
    pub note: Option<String>,
    pub after: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct CheckpointArgs {
    pub session: String,
    pub goal: Option<String>,
    pub decisions: Option<Vec<String>>,
    pub pending: Option<Vec<String>>,
    #[serde(rename = "refreshFreshness")]
    pub refresh_freshness: Option<bool>,
}
impl CheckpointArgs {
    pub fn has_updates(&self) -> bool {
        self.goal.is_some() || self.decisions.is_some() || self.pending.is_some()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum HandoverKind {
    #[default]
    Summary,
    Adr,
    PrBody,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct HandoverArgs {
    pub session: String,
    pub kind: Option<HandoverKind>,
    #[serde(rename = "targetFile")]
    pub target_file: Option<String>,
    #[serde(rename = "expectedHash")]
    pub expected_hash: Option<String>,
}
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ProcessListFilter {
    Active,
    Recent,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ProcessStatusArgs {
    pub pid: Option<u32>,
    #[serde(rename = "runId")]
    pub run_id: Option<String>,
    pub list: Option<ProcessListFilter>,
    pub limit: Option<usize>,
    pub port: Option<u16>,
    pub host: Option<String>,
    #[serde(rename = "healthUrl")]
    pub health_url: Option<String>,
    #[serde(rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
    pub session: Option<String>,
}

/// The application has one entry point; transports cannot bypass its admission checks.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GitStatusArgs {
    #[serde(rename = "includeUntracked")]
    pub include_untracked: Option<bool>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GitDiffArgs {
    pub path: String,
    pub staged: Option<bool>,
    #[serde(rename = "maxLines")]
    pub max_lines: Option<usize>,
    #[serde(rename = "maxChars")]
    pub max_chars: Option<usize>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsArgs {
    pub files: Option<Vec<String>>,
    #[serde(rename = "maxFiles")]
    pub max_files: Option<usize>,
    #[serde(rename = "buildScope")]
    pub build_scope: Option<String>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataQueryArgs {
    pub path: Option<String>,
    pub data: Option<Value>,
    /// Stored JSON root from a previous detail, subject to the same TTL.
    #[serde(rename = "resultRef")]
    pub result_ref: Option<String>,
    pub query: String,
    #[serde(rename = "maxChars")]
    pub max_chars: Option<usize>,
    pub session: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum BatchTool {
    #[serde(rename = "harness_config")]
    Config,
    #[serde(rename = "harness_read")]
    Read,
    #[serde(rename = "harness_search")]
    Search,
    #[serde(rename = "harness_outline")]
    Outline,
    #[serde(rename = "harness_read_symbol")]
    ReadSymbol,
    #[serde(rename = "harness_detail")]
    Detail,
    #[serde(rename = "harness_git_status")]
    GitStatus,
    #[serde(rename = "harness_git_diff")]
    GitDiff,
    #[serde(rename = "harness_sql")]
    Sql,
    #[serde(rename = "harness_xlsx")]
    Xlsx,
    #[serde(rename = "harness_svg")]
    Svg,
}
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct BatchJob {
    pub tool: BatchTool,
    pub arguments: serde_json::Map<String, Value>,
}
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct BatchArgs {
    pub jobs: Vec<BatchJob>,
    pub session: Option<String>,
}
impl BatchJob {
    pub fn into_request(mut self, session: Option<&str>) -> Result<Request, crate::Failure> {
        if let Some(session) = session {
            self.arguments
                .insert("session".into(), Value::String(session.into()));
        }
        let value = Value::Object(self.arguments);
        let decoded = match self.tool {
            BatchTool::Config => serde_json::from_value(value).map(Request::Config),
            BatchTool::Read => serde_json::from_value(value).map(Request::Read),
            BatchTool::Search => serde_json::from_value(value).map(Request::Search),
            BatchTool::Outline => serde_json::from_value(value).map(Request::Outline),
            BatchTool::ReadSymbol => serde_json::from_value(value).map(Request::ReadSymbol),
            BatchTool::Detail => serde_json::from_value(value).map(Request::Detail),
            BatchTool::GitStatus => serde_json::from_value(value).map(Request::GitStatus),
            BatchTool::GitDiff => serde_json::from_value(value).map(Request::GitDiff),
            BatchTool::Sql => serde_json::from_value::<crate::sql::SqlArgs>(value)
                .map(crate::sql::SqlArgs::into_request),
            BatchTool::Xlsx => serde_json::from_value::<crate::xlsx::XlsxArgs>(value)
                .map(crate::xlsx::XlsxArgs::into_request),
            BatchTool::Svg => serde_json::from_value(value).map(Request::Svg),
        };
        decoded.map_err(|_| ("ARGUMENT", "Invalid batch child arguments; no jobs started"))
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExploreArgs {
    pub query: String,
    /// Summary returns signatures and graph counts; detail retains the graph.
    pub view: Option<ExploreView>,
    #[serde(rename = "filePath")]
    pub file_path: Option<String>,
    pub container: Option<String>,
    /// CodeGraph node id from exploration candidates, not a local AST symbol id.
    #[serde(rename = "symbolId")]
    pub symbol_id: Option<String>,
    /// Explicit authorization to write the existing native index before querying.
    /// Omitted/false preserves read-only exploration; no index is created implicitly.
    #[serde(rename = "syncIndex")]
    pub sync_index: Option<bool>,
    pub symbol: Option<String>,
    #[serde(rename = "maxFiles")]
    pub max_files: Option<usize>,
    #[serde(rename = "maxChars")]
    pub max_chars: Option<usize>,
    /// Indexed traversal only: defaults to both directions and one hop.
    pub direction: Option<ExploreDirection>,
    #[serde(rename = "maxDepth")]
    pub max_depth: Option<usize>,
    /// Reject results unless all contributing files match indexed hashes.
    /// This does not certify full index coverage or freshness.
    #[serde(rename = "requireMatchingFiles")]
    pub require_matching_files: Option<bool>,
    #[serde(alias = "sessionId")]
    pub session: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExploreView {
    Summary,
    Full,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExploreDirection {
    Inbound,
    Outbound,
    Both,
}

pub enum Request {
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
    Move(MoveArgs),
    Outline(OutlineArgs),
    ReadSymbol(ReadSymbolArgs),
    Tasks(TasksArgs),
    Checkpoint(CheckpointArgs),
    GitStatus(GitStatusArgs),
    GitDiff(GitDiffArgs),
    SqlStatus(crate::sql::SqlStatusArgs),
    SqlSearch(crate::sql::SqlSearchArgs),
    SqlDescribe(crate::sql::SqlDescribeArgs),
    SqlQuery(crate::sql::SqlQueryArgs),
    XlsxInspect(crate::xlsx::XlsxInspectArgs),
    XlsxRead(crate::xlsx::XlsxReadArgs),
    Svg(crate::svg::SvgArgs),
    Handover(HandoverArgs),
    ProcessStatus(ProcessStatusArgs),
    Diagnostics(DiagnosticsArgs),
    DataQuery(DataQueryArgs),
}
impl Request {
    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            Self::Replace(_)
                | Self::Move(_)
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
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Explore(_) => "harness_explore",
            Self::ExternalScope(_) => "harness_external_scope",
            Self::Config(_) => "harness_config",
            Self::Logs(_) => "harness_logs",
            Self::Batch(_) => "harness_batch",
            Self::Doctor(_) => "harness_doctor",
            Self::Bootstrap(_) => "harness_bootstrap",
            Self::Read(_) => "harness_read",
            Self::Detail(_) => "harness_detail",
            Self::Search(_) => "harness_search",
            Self::Verify(_) => "harness_verify",
            Self::Replace(_) => "harness_replace",
            Self::Move(_) => "harness_move",
            Self::Outline(_) => "harness_outline",
            Self::ReadSymbol(_) => "harness_read_symbol",
            Self::Tasks(_) => "harness_tasks",
            Self::Checkpoint(_) => "harness_checkpoint",
            Self::GitStatus(_) => "harness_git_status",
            Self::GitDiff(_) => "harness_git_diff",
            Self::SqlStatus(_) | Self::SqlSearch(_) | Self::SqlDescribe(_) | Self::SqlQuery(_) => {
                "harness_sql"
            }
            Self::XlsxInspect(_) | Self::XlsxRead(_) => "harness_xlsx",
            Self::Svg(_) => "harness_svg",
            Self::Handover(_) => "harness_handover",
            Self::ProcessStatus(_) => "harness_process_status",
            Self::Diagnostics(_) => "harness_diagnostics",
            Self::DataQuery(_) => "harness_data_query",
        }
    }
    pub fn arguments(&self) -> Value {
        match self {
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
            Self::Move(args) => serde_json::to_value(args),
            Self::Outline(args) => serde_json::to_value(args),
            Self::ReadSymbol(args) => serde_json::to_value(args),
            Self::Tasks(args) => serde_json::to_value(args),
            Self::Checkpoint(args) => serde_json::to_value(args),
            Self::GitStatus(args) => serde_json::to_value(args),
            Self::GitDiff(args) => serde_json::to_value(args),
            Self::SqlStatus(args) => serde_json::to_value(args),
            Self::SqlSearch(args) => serde_json::to_value(args),
            Self::SqlDescribe(args) => serde_json::to_value(args),
            Self::SqlQuery(args) => serde_json::to_value(args),
            Self::XlsxInspect(args) => serde_json::to_value(args),
            Self::XlsxRead(args) => serde_json::to_value(args),
            Self::Svg(args) => serde_json::to_value(args),
            Self::Handover(args) => serde_json::to_value(args),
            Self::ProcessStatus(args) => serde_json::to_value(args),
            Self::Diagnostics(args) => serde_json::to_value(args),
            Self::DataQuery(args) => serde_json::to_value(args),
        }
        .expect("request arguments serialize")
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DoctorArgs {
    /// Defaults to false: locating tools never starts a subprocess.
    pub versions: Option<bool>,
    pub connection: Option<bool>,
    pub session: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ConfigArgs {
    pub path: String,
    pub section: String,
    #[serde(rename = "maxChars")]
    pub max_chars: Option<usize>,
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    pub session: Option<String>,
    /// When true, extracts the raw section text with IIS/ASP.NET heuristic regex redaction
    /// (preserving keys and attributes, redacting sensitive secrets like password/pwd/token/connectionString).
    pub raw: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExternalScopeAction {
    Grant,
    List,
    Revoke,
}
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ExternalScopeArgs {
    pub action: ExternalScopeAction,
    pub session: String,
    pub root: Option<String>,
    #[serde(rename = "userAuthorized")]
    pub user_authorized: Option<bool>,
    #[serde(rename = "scopeId")]
    pub scope_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct LogsArgs {
    pub limit: Option<usize>,
    pub level: Option<String>,
    pub tool: Option<String>,
    #[serde(rename = "slowOnly")]
    pub slow_only: Option<bool>,
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
    pub after: Option<u64>,
    pub session: Option<String>,
}
