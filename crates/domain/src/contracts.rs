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

/// Local AST navigation; graph relationships remain in harness_explore.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SymbolsArgs {
    Outline(OutlineArgs),
    Read(ReadSymbolArgs),
}

impl SymbolsArgs {
    pub fn into_request(self) -> Request {
        match self {
            Self::Outline(args) => Request::Outline(args),
            Self::Read(args) => Request::ReadSymbol(args),
        }
    }
}

#[cfg(test)]
mod symbols_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_selectors_views_scope_and_session_in_both_actions() {
        for value in [
            json!({"action":"outline","path":"src/lib.rs","query":"target","maxChars":2500,"externalScope":"scope","session":"s"}),
            json!({"action":"read","path":"src/lib.rs","symbol":"target","container":"mod","symbolId":"0123456789abcdef01234567","view":"full","externalScope":"scope","session":"s"}),
            json!({"action":"read","path":"src/lib.rs","symbol":"target","view":"signature","session":"s"}),
        ] {
            let request = serde_json::from_value::<SymbolsArgs>(value.clone())
                .unwrap()
                .into_request();
            assert_eq!(request.name(), "harness_symbols");
            assert!(!request.is_mutating());
            let restored = request.arguments();
            for (key, expected) in value.as_object().unwrap() {
                assert_eq!(&restored[key], expected);
            }
            assert!(serde_json::from_value::<SymbolsArgs>(restored).is_ok());
        }
        let legacy_session: SymbolsArgs = serde_json::from_value(
            json!({"action":"read","path":"x.rs","symbol":"x","sessionId":"s"}),
        )
        .unwrap();
        assert_eq!(legacy_session.into_request().arguments()["session"], "s");
    }

    #[test]
    fn rejects_incomplete_or_cross_action_arguments_and_supports_batch() {
        for value in [
            json!({}),
            json!({"action":"outline"}),
            json!({"action":"read","path":"x.rs"}),
            json!({"action":"read","path":"x.rs","name":"x"}),
            json!({"action":"outline","path":"x.rs","view":"full"}),
            json!({"action":"read","path":"x.rs","symbol":"x","query":"y"}),
            json!({"action":"delete","path":"x.rs"}),
        ] {
            assert!(serde_json::from_value::<SymbolsArgs>(value).is_err());
        }
        for action in ["outline", "read"] {
            let mut value =
                json!({"tool":"harness_symbols","arguments":{"action":action,"path":"x.rs"}});
            if action == "read" {
                value["arguments"]["symbol"] = json!("x");
            }
            let request = serde_json::from_value::<BatchJob>(value)
                .unwrap()
                .into_request(Some("s"))
                .unwrap();
            assert_eq!(request.arguments()["action"], action);
            assert_eq!(request.arguments()["session"], "s");
        }
    }

    #[cfg(feature = "schema")]
    #[test]
    fn symbols_schema_requires_path_and_symbol_only_for_read() {
        let schema = serde_json::to_value(schemars::schema_for!(SymbolsArgs)).unwrap();
        assert_eq!(schema["type"], "object");
        let branches = schema["oneOf"].as_array().unwrap();
        assert_eq!(branches.len(), 2);
        for branch in branches {
            let required = branch["required"].as_array().unwrap();
            assert!(required.contains(&json!("path")));
            assert_eq!(
                required.contains(&json!("symbol")),
                branch["properties"]["action"]["const"] == "read"
            );
        }
    }
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

/// Read-only Git operations with action-specific arguments.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GitArgs {
    Status(GitStatusArgs),
    Diff(GitDiffArgs),
}

impl GitArgs {
    pub fn into_request(self) -> Request {
        match self {
            Self::Status(args) => Request::GitStatus(args),
            Self::Diff(args) => Request::GitDiff(args),
        }
    }
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
    #[serde(rename = "harness_symbols")]
    Symbols,
    #[serde(rename = "harness_detail")]
    Detail,
    #[serde(rename = "harness_git")]
    Git,
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
            BatchTool::Symbols => {
                serde_json::from_value::<SymbolsArgs>(value).map(SymbolsArgs::into_request)
            }
            BatchTool::Detail => serde_json::from_value(value).map(Request::Detail),
            BatchTool::Git => serde_json::from_value::<GitArgs>(value).map(GitArgs::into_request),
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
            Self::Outline(_) | Self::ReadSymbol(_) => "harness_symbols",
            Self::Tasks(_) => "harness_tasks",
            Self::Checkpoint(_) => "harness_checkpoint",
            Self::GitStatus(_) | Self::GitDiff(_) => "harness_git",
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
            Self::Outline(args) => serde_json::to_value(args).map(|mut value| {
                value["action"] = Value::String("outline".into());
                value
            }),
            Self::ReadSymbol(args) => serde_json::to_value(args).map(|mut value| {
                value["action"] = Value::String("read".into());
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

#[cfg(test)]
mod git_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unified_git_preserves_action_options_and_read_only_admission() {
        for value in [
            json!({"action":"status","includeUntracked":false,"offset":5,"limit":10,"session":"s"}),
            json!({"action":"diff","path":"deleted.txt","staged":true,"maxLines":20,"maxChars":900,"session":"s"}),
        ] {
            let request = serde_json::from_value::<GitArgs>(value.clone())
                .unwrap()
                .into_request();
            assert_eq!(request.name(), "harness_git");
            assert!(!request.is_mutating());
            let restored = request.arguments();
            for (key, expected) in value.as_object().unwrap() {
                assert_eq!(&restored[key], expected);
            }
            assert!(serde_json::from_value::<GitArgs>(restored).is_ok());
        }
    }

    #[test]
    fn unified_git_rejects_missing_or_cross_action_fields_and_decodes_batches() {
        for value in [
            json!({}),
            json!({"action":"diff"}),
            json!({"action":"commit"}),
            json!({"action":"status","path":"x"}),
            json!({"action":"diff","path":"x","limit":5}),
            json!({"action":"status","unknown":true}),
        ] {
            assert!(serde_json::from_value::<GitArgs>(value).is_err());
        }
        let job: BatchJob = serde_json::from_value(
            json!({"tool":"harness_git","arguments":{"action":"diff","path":"x"}}),
        )
        .unwrap();
        let request = job.into_request(Some("session")).unwrap();
        assert_eq!(request.arguments()["action"], "diff");
        assert_eq!(request.arguments()["session"], "session");
        assert!(serde_json::from_value::<BatchTool>(json!("harness_git_status")).is_err());
        let job: BatchJob =
            serde_json::from_value(json!({"tool":"harness_git","arguments":{"action":"diff"}}))
                .unwrap();
        assert_eq!(job.into_request(None).err().unwrap().0, "ARGUMENT");
    }

    #[cfg(feature = "schema")]
    #[test]
    fn git_schema_has_root_object_and_action_specific_branches() {
        let schema = serde_json::to_value(schemars::schema_for!(GitArgs)).unwrap();
        assert_eq!(schema["type"], "object");
        let branches = schema["oneOf"].as_array().unwrap();
        assert_eq!(branches.len(), 2);
        let diff = branches
            .iter()
            .find(|branch| branch["properties"]["action"]["const"] == "diff")
            .unwrap();
        assert!(
            diff["required"]
                .as_array()
                .unwrap()
                .contains(&json!("path"))
        );
    }
}
