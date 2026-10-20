use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct BootstrapArgs {
    /// Opt in to a compact initial catalog; project rules and safety notices remain inline.
    #[serde(default)]
    pub compact: bool,
    /// Reuse data.session from bootstrap. sessionId is accepted for compatibility.
    #[serde(alias = "sessionId")]
    pub session: Option<String>,
    #[serde(rename = "includeRules")]
    pub include_rules: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[serde(try_from = "ReadInput")]
pub struct ReadArgs {
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    pub path: String,
    pub start: usize,
    pub end: usize,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("oneOf" = [
    {"required":["end"],"not":{"required":["lines"]}},
    {"required":["lines"],"not":{"required":["end"]}}
])))]
#[cfg_attr(feature = "schema", schemars(transform = read_alias_schema))]
struct ReadInput {
    #[serde(rename = "externalScope")]
    external_scope: Option<String>,
    /// File path; file and target_path are accepted aliases. Supply only one.
    #[serde(alias = "target_path", alias = "file")]
    path: String,
    start: usize,
    end: Option<usize>,
    /// Number of lines starting at start; mutually exclusive with end.
    lines: Option<usize>,
    #[serde(alias = "sessionId")]
    session: Option<String>,
}

#[cfg(feature = "schema")]
fn add_schema_alias(schema: &mut schemars::Schema, canonical: &str, alias: &str) {
    let object = schema.as_object_mut().expect("object schema");
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        if let Some(property) = properties.get(canonical).cloned() {
            properties.insert(alias.to_string(), property);
        }
    }
    let required = object.get_mut("required").and_then(Value::as_array_mut);
    let was_required = required.is_some_and(|required| {
        let present = required.iter().any(|value| value == canonical);
        required.retain(|value| value != canonical);
        present
    });
    let constraints = object
        .entry("allOf")
        .or_insert_with(|| serde_json::json!([]));
    let constraints = constraints.as_array_mut().expect("allOf array");
    constraints.push(serde_json::json!({"not":{"required":[canonical,alias]}}));
    if was_required {
        constraints
            .push(serde_json::json!({"anyOf":[{"required":[canonical]},{"required":[alias]}]}));
    }
}

#[cfg(feature = "schema")]
fn read_alias_schema(schema: &mut schemars::Schema) {
    // target_path predates file; one selector is required and multiple selectors are invalid.
    add_schema_alias(schema, "path", "file");
    add_schema_alias(schema, "path", "target_path");
    let object = schema.as_object_mut().expect("object schema");
    let constraints = object.get_mut("allOf").unwrap().as_array_mut().unwrap();
    // Replace the initial two-way requirement with all three accepted names.
    constraints.retain(|value| value.get("anyOf").is_none());
    constraints.push(serde_json::json!({"oneOf":[{"required":["path"]},{"required":["file"]},{"required":["target_path"]}]}));
}

#[cfg(feature = "schema")]
fn search_alias_schema(schema: &mut schemars::Schema) {
    add_schema_alias(schema, "scope", "path");
    add_schema_alias(schema, "globs", "includes");
}

#[cfg(feature = "schema")]
fn replace_alias_schema(schema: &mut schemars::Schema) {
    add_schema_alias(schema, "hash", "baseHash");
}

impl TryFrom<ReadInput> for ReadArgs {
    type Error = String;
    fn try_from(input: ReadInput) -> Result<Self, Self::Error> {
        let end = match (input.end, input.lines) {
            (Some(end), None) => end,
            (None, Some(count)) if count > 0 && count <= 300 => input
                .start
                .checked_add(count - 1)
                .ok_or("read range overflow")?,
            (Some(_), Some(_)) => return Err("use either end or lines, not both".into()),
            _ => return Err("provide end or lines (1..300)".into()),
        };
        Ok(Self {
            external_scope: input.external_scope,
            path: input.path,
            start: input.start,
            end,
            session: input.session,
        })
    }
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

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(transform = search_alias_schema))]
pub struct SearchArgs {
    #[serde(rename = "externalScope")]
    pub external_scope: Option<String>,
    pub kind: Kind,
    #[serde(default, alias = "pattern")]
    pub query: String,
    #[serde(rename = "minLines")]
    pub min_lines: Option<usize>,
    /// File patterns; includes is accepted as an alias. Supply only one.
    #[serde(alias = "includes")]
    pub globs: Option<Vec<String>>,
    /// Search file or directory; path is an alias. Supply only one, e.g. {"kind":"text","query":"Controller","scope":"Controllers"}.
    #[serde(alias = "path")]
    pub scope: Option<String>,
    pub limit: Option<usize>,
    pub session: Option<String>,
}

impl<'de> Deserialize<'de> for SearchArgs {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            #[serde(rename = "externalScope")]
            external_scope: Option<String>,
            kind: Kind,
            #[serde(default, alias = "pattern")]
            query: String,
            #[serde(rename = "minLines")]
            min_lines: Option<usize>,
            #[serde(alias = "includes")]
            globs: Option<Vec<String>>,
            #[serde(alias = "path")]
            scope: Option<String>,
            limit: Option<usize>,
            session: Option<String>,
        }
        let mut value = Value::deserialize(deserializer)?;
        strip_inocuous(&mut value);
        let input = Input::deserialize(value).map_err(|error| {
            serde::de::Error::custom(format!(
                "{error}; use scope or its alias path (only one), e.g. {{\"kind\":\"text\",\"query\":\"Controller\",\"scope\":\"Controllers\"}}"
            ))
        })?;
        Ok(Self {
            external_scope: input.external_scope,
            kind: input.kind,
            query: input.query,
            min_lines: input.min_lines,
            globs: input.globs,
            scope: input.scope,
            limit: input.limit,
            session: input.session,
        })
    }
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
    /// Rust package name, passed as a single cargo -p argument.
    pub package: Option<String>,
    pub action: Option<VerifyAction>,
    pub session: Option<String>,
}

/// Harmless envelope keys some clients add; never part of these tool arguments.
const INOCUOUS_FIELDS: &[&str] = &["action", "object", "objectType"];

fn strip_inocuous(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        for key in INOCUOUS_FIELDS {
            object.remove(*key);
        }
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[serde(try_from = "ReplaceInput")]
pub struct ReplaceArgs {
    pub path: String,
    /// Override UTF-8 BOM. Omitted preserves an existing BOM and enables it for Razor on Windows.
    pub bom: Option<bool>,
    /// Current source SHA-256; baseHash is accepted as an alias. Supply only one.
    /// Optional on the wire; an omitted hash is bound to the source read during this edit.
    #[serde(default)]
    pub hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunks: Option<Vec<ReplaceChunk>>,
    #[serde(default, rename = "oldText")]
    pub old_text: String,
    #[serde(default, rename = "newText")]
    pub new_text: String,
    /// Explicitly permit UTF-8 text when no syntax parser is available.
    #[serde(default, rename = "allowUnvalidatedText")]
    pub allow_unvalidated_text: bool,
    /// When true, preview unified diff and syntax diagnostics without publishing.
    #[serde(default)]
    pub preview: bool,
    pub session: Option<String>,
}

impl<'de> serde::Deserialize<'de> for ReplaceArgs {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut value = Value::deserialize(deserializer)?;
        strip_inocuous(&mut value);
        let input: ReplaceInput = serde_json::from_value(value).map_err(|_| {
            serde::de::Error::custom(
                "Invalid harness_replace arguments: path plus oldText/newText (or chunks); hash/baseHash optional",
            )
        })?;
        Self::try_from(input).map_err(serde::de::Error::custom)
    }
}

impl Default for ReplaceArgs {
    fn default() -> Self {
        Self {
            path: String::new(),
            bom: None,
            hash: String::new(),
            chunks: None,
            old_text: String::new(),
            new_text: String::new(),
            allow_unvalidated_text: false,
            preview: false,
            session: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MoveArgs {
    pub from: String,
    pub to: String,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct WriteArgs {
    #[serde(alias = "target_path")]
    pub path: String,
    pub content: String,
    /// Override UTF-8 BOM. Omitted preserves an existing BOM; new .cshtml files on Windows use BOM.
    pub bom: Option<bool>,
    /// Create missing parent directories within the workspace; never follow links.
    #[serde(default, rename = "createParents")]
    pub create_parents: bool,
    /// Explicitly permit UTF-8 text when no syntax parser is available.
    #[serde(default, rename = "allowUnvalidatedText")]
    pub allow_unvalidated_text: bool,
    /// When present, turns the write into an atomic whole-file replace of an
    /// existing file whose current SHA-256 must equal this value.
    #[serde(default, rename = "expectedHash")]
    pub expected_hash: Option<String>,
    /// When true, allows overwriting an existing file with new content atomically.
    #[serde(default)]
    pub overwrite: bool,
    #[serde(alias = "sessionId")]
    pub session: Option<String>,
}

impl Serialize for ReplaceArgs {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("path", &self.path)?;
        if let Some(bom) = self.bom {
            map.serialize_entry("bom", &bom)?;
        }
        if !self.hash.is_empty() {
            map.serialize_entry("hash", &self.hash)?;
        }
        if let Some(chunks) = &self.chunks {
            map.serialize_entry("chunks", chunks)?;
        } else {
            map.serialize_entry("oldText", &self.old_text)?;
            map.serialize_entry("newText", &self.new_text)?;
        }
        if self.allow_unvalidated_text {
            map.serialize_entry("allowUnvalidatedText", &true)?;
        }
        if self.preview {
            map.serialize_entry("preview", &true)?;
        }
        map.serialize_entry("session", &self.session)?;
        map.end()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
#[cfg_attr(feature = "schema", schemars(transform = replace_alias_schema))]
struct ReplaceInput {
    #[serde(alias = "target_path")]
    path: String,
    bom: Option<bool>,
    /// Current source SHA-256. baseHash is an accepted alias; supply only one name.
    #[serde(default, alias = "baseHash")]
    hash: String,
    chunks: Option<Vec<ReplaceChunk>>,
    #[serde(rename = "oldText")]
    old_text: Option<String>,
    #[serde(rename = "newText")]
    new_text: Option<String>,
    #[serde(default, rename = "allowUnvalidatedText")]
    allow_unvalidated_text: bool,
    #[serde(default)]
    preview: bool,
    #[serde(alias = "sessionId")]
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
            bom: input.bom,
            hash: input.hash,
            chunks: input.chunks,
            old_text,
            new_text,
            allow_unvalidated_text: input.allow_unvalidated_text,
            preview: input.preview,
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
#[serde(deny_unknown_fields)]
pub struct VueExtractPreviewArgs {
    pub path: String,
    #[serde(rename = "expectedHash")]
    pub expected_hash: String,
    /// Current template element ID returned by Vue outline.
    #[serde(rename = "symbolId")]
    pub symbol_id: String,
    #[serde(rename = "targetPath")]
    pub target_path: String,
    #[serde(rename = "componentName")]
    pub component_name: String,
    #[serde(alias = "sessionId")]
    pub session: Option<String>,
}

/// Local AST navigation and read-only Vue extraction planning.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SymbolsArgs {
    Outline(OutlineArgs),
    Read(ReadSymbolArgs),
    ExtractPreview(VueExtractPreviewArgs),
}

impl SymbolsArgs {
    pub fn into_request(self) -> Request {
        match self {
            Self::Outline(args) => Request::Outline(args),
            Self::Read(args) => Request::ReadSymbol(args),
            Self::ExtractPreview(args) => Request::VueExtractPreview(args),
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
            json!({"action":"extract_preview","path":"Parent.vue","expectedHash":"a".repeat(64),"symbolId":"b".repeat(24),"targetPath":"Child.vue","componentName":"ChildView","session":"s"}),
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
            json!({"action":"extract_preview","path":"Parent.vue"}),
            json!({"action":"extract_preview","path":"Parent.vue","expectedHash":"a".repeat(64),"symbolId":"b".repeat(24),"targetPath":"Child.vue","componentName":"ChildView","apply":true}),
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
        assert_eq!(branches.len(), 3);
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
#[serde(try_from = "CheckpointInput")]
pub struct CheckpointArgs {
    pub session: String,
    pub goal: Option<String>,
    pub decisions: Option<Vec<String>>,
    pub pending: Option<Vec<String>>,
    #[serde(rename = "refreshFreshness")]
    pub refresh_freshness: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct CheckpointInput {
    session: String,
    goal: Option<String>,
    decisions: Option<Vec<String>>,
    pending: Option<Vec<String>>,
    #[serde(rename = "refreshFreshness")]
    refresh_freshness: Option<bool>,
    /// Alternate envelope for goal, decisions and pending. Duplicate fields are rejected.
    data: Option<CheckpointData>,
}

#[derive(Debug, Default, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct CheckpointData {
    goal: Option<String>,
    decisions: Option<Vec<String>>,
    pending: Option<Vec<String>>,
}

impl TryFrom<CheckpointInput> for CheckpointArgs {
    type Error = String;
    fn try_from(input: CheckpointInput) -> Result<Self, Self::Error> {
        let data = input.data.unwrap_or_default();
        fn merge<T>(root: Option<T>, nested: Option<T>, field: &str) -> Result<Option<T>, String> {
            if root.is_some() && nested.is_some() {
                return Err(format!(
                    "duplicate checkpoint field {field}: supply it at the root or in data, not both"
                ));
            }
            Ok(root.or(nested))
        }
        Ok(Self {
            session: input.session,
            goal: merge(input.goal, data.goal, "goal")?,
            decisions: merge(input.decisions, data.decisions, "decisions")?,
            pending: merge(input.pending, data.pending, "pending")?,
            refresh_freshness: input.refresh_freshness,
        })
    }
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

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SessionTasksArgs {
    pub session: String,
    pub subaction: Option<TasksAction>,
    pub phase: Option<String>,
    pub status: Option<String>,
    pub note: Option<String>,
    pub after: Option<u64>,
    pub limit: Option<usize>,
}

impl SessionTasksArgs {
    pub fn into_tasks_args(self) -> TasksArgs {
        let action = self.subaction.or_else(|| {
            if self.phase.is_some() || self.status.is_some() || self.note.is_some() {
                Some(TasksAction::Update)
            } else if self.after.is_some() || self.limit.is_some() {
                Some(TasksAction::Timeline)
            } else {
                Some(TasksAction::Status)
            }
        });
        TasksArgs {
            session: self.session,
            action,
            phase: self.phase,
            status: self.status,
            note: self.note,
            after: self.after,
            limit: self.limit,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SessionStatusArgs {
    pub session: String,
}

/// Unified session and workflow orchestration.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SessionArgs {
    Checkpoint(CheckpointArgs),
    Tasks(SessionTasksArgs),
    Handover(HandoverArgs),
    Status(SessionStatusArgs),
}

impl SessionArgs {
    pub fn into_request(self) -> Request {
        match self {
            Self::Checkpoint(args) => Request::Checkpoint(args),
            Self::Tasks(args) => Request::Tasks(args.into_tasks_args()),
            Self::Handover(args) => Request::Handover(args),
            Self::Status(args) => Request::Checkpoint(CheckpointArgs {
                session: args.session,
                goal: None,
                decisions: None,
                pending: None,
                refresh_freshness: Some(false),
            }),
        }
    }
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
    #[serde(rename = "harness_read", alias = "read")]
    Read,
    #[serde(rename = "harness_search", alias = "search")]
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
    /// view=impact only: maximum displayed callers, 1..100; defaults to 20.
    pub limit: Option<usize>,
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
    Impact,
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
    Outline(OutlineArgs),
    ReadSymbol(ReadSymbolArgs),
    VueExtractPreview(VueExtractPreviewArgs),
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
            Self::Outline(_) | Self::ReadSymbol(_) | Self::VueExtractPreview(_) => {
                "harness_symbols"
            }
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
            Self::Diagnostics(_) => "harness_doctor",
            Self::DataQuery(_) => "harness_data_query",
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

#[derive(Debug, Deserialize, Serialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DoctorEnvArgs {
    /// Defaults to false: locating tools never starts a subprocess.
    pub versions: Option<bool>,
    pub connection: Option<bool>,
    pub session: Option<String>,
}

pub type DoctorArgs = DoctorEnvArgs;

/// Unified system diagnostics, file syntax checks, and log inspection.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DoctorToolArgs {
    Env(DoctorEnvArgs),
    Syntax(DiagnosticsArgs),
    Logs(LogsArgs),
}

impl<'de> serde::Deserialize<'de> for DoctorToolArgs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let action = value
            .as_object_mut()
            .and_then(|obj| obj.remove("action"))
            .and_then(|v| v.as_str().map(String::from));
        match action.as_deref() {
            Some("syntax") => serde_json::from_value(value)
                .map(Self::Syntax)
                .map_err(serde::de::Error::custom),
            Some("logs") => serde_json::from_value(value)
                .map(Self::Logs)
                .map_err(serde::de::Error::custom),
            Some("env") | None => serde_json::from_value(value)
                .map(Self::Env)
                .map_err(serde::de::Error::custom),
            Some(unknown) => Err(serde::de::Error::custom(format!(
                "unknown action: {unknown}"
            ))),
        }
    }
}

impl DoctorToolArgs {
    pub fn into_request(self) -> Request {
        match self {
            Self::Env(args) => Request::Doctor(args),
            Self::Syntax(args) => Request::Diagnostics(args),
            Self::Logs(args) => Request::Logs(args),
        }
    }
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
mod compatibility_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inocuous_envelope_fields_are_tolerated_for_edit_and_search() {
        use crate::hash;
        let replace: ReplaceArgs = serde_json::from_value(json!({
            "action":"harness_replace","object":"file","objectType":"text",
            "path":"a.txt","oldText":"a","newText":"b","baseHash":hash(b"abc")
        }))
        .unwrap();
        assert_eq!(replace.old_text, "a");
        assert_eq!(replace.hash, hash(b"abc"));
        let search: SearchArgs = serde_json::from_value(json!({
            "action":"harness_search","object":"files",
            "kind":"files","query":"a","scope":"."  })).unwrap();
        assert_eq!(search.query, "a");
        assert!(
            serde_json::from_value::<ReplaceArgs>(json!({
                "path":"a.txt","oldText":"a","newText":"b","unknown":true})).is_err()
        );
    }

    #[test]
    fn search_aliases_normalize_and_reject_duplicates_with_example() {
        let args: SearchArgs = serde_json::from_value(json!({
            "kind":"text", "query":"Controller", "path":"Controllers", "includes":["*.cs"]
        }))
        .unwrap();
        assert_eq!(args.scope.as_deref(), Some("Controllers"));
        assert_eq!(args.globs.unwrap(), vec!["*.cs"]);
        for input in [
            json!({"kind":"text","path":"a","scope":"b"}),
            json!({"kind":"text","scope":42}),
            json!({"kind":"text","includes":["*.cs"],"globs":["*.js"]}),
        ] {
            let error = serde_json::from_value::<SearchArgs>(input)
                .unwrap_err()
                .to_string();
            assert!(error.contains("\"scope\":\"Controllers\""), "{error}");
        }
    }

    #[test]
    fn read_file_alias_normalizes_and_rejects_duplicate_selectors() {
        let args: ReadArgs =
            serde_json::from_value(json!({"file":"view.cshtml","start":1,"lines":5})).unwrap();
        assert_eq!(args.path, "view.cshtml");
        assert_eq!(args.end, 5);
        let error =
            serde_json::from_value::<ReadArgs>(json!({"file":"a","path":"b","start":1,"end":5}))
                .unwrap_err()
                .to_string();
        assert!(error.contains("duplicate field `path`"), "{error}");
    }

    #[test]
    fn checkpoint_data_normalizes_dispatches_and_rejects_conflicts() {
        let args: SessionArgs = serde_json::from_value(json!({
            "action":"checkpoint","session":"s","data":{"goal":"fix","decisions":["aliases"],"pending":["test"]}
        })).unwrap();
        let request = args.into_request();
        assert!(request.is_mutating());
        let canonical = request.arguments();
        assert_eq!(canonical["goal"], "fix");
        assert_eq!(canonical["decisions"], json!(["aliases"]));
        assert!(canonical.get("data").is_none());
        assert!(
            serde_json::from_value::<SessionArgs>(json!({
                "action":"checkpoint","session":"s","goal":"a","data":{"goal":"b"}
            }))
            .unwrap_err()
            .to_string()
            .contains("duplicate checkpoint field goal")
        );
        assert!(
            serde_json::from_value::<SessionArgs>(json!({
                "action":"checkpoint","session":"s","data":{"unknown":true}
            }))
            .is_err()
        );
    }

    #[cfg(feature = "schema")]
    #[test]
    fn input_schemas_advertise_aliases_and_checkpoint_envelope() {
        let search = serde_json::to_value(schemars::schema_for!(SearchArgs)).unwrap();
        assert!(search["properties"].get("path").is_some());
        assert!(search["properties"].get("includes").is_some());
        let read = serde_json::to_value(schemars::schema_for!(ReadArgs)).unwrap();
        assert!(read["properties"].get("file").is_some());
        assert!(
            !read["required"]
                .as_array()
                .unwrap()
                .contains(&json!("path"))
        );
        let checkpoint = serde_json::to_value(schemars::schema_for!(CheckpointArgs)).unwrap();
        assert!(checkpoint["properties"].get("data").is_some());
    }
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

#[cfg(test)]
mod session_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_args_deserializes_and_dispatches_all_actions() {
        // checkpoint
        let ckpt: SessionArgs = serde_json::from_value(json!({
            "action": "checkpoint",
            "session": "s1",
            "goal": "build session tool",
            "decisions": ["unified"],
            "pending": ["tests"]
        }))
        .unwrap();
        let req = ckpt.into_request();
        assert_eq!(req.name(), "harness_checkpoint");
        assert!(req.is_mutating());

        // tasks update
        let tasks_up: SessionArgs = serde_json::from_value(json!({
            "action": "tasks",
            "session": "s1",
            "phase": "execution",
            "status": "in_progress",
            "note": "implementing session"
        }))
        .unwrap();
        let req = tasks_up.into_request();
        assert_eq!(req.name(), "harness_tasks");
        assert!(req.is_mutating());

        // tasks timeline
        let tasks_tl: SessionArgs = serde_json::from_value(json!({
            "action": "tasks",
            "session": "s1",
            "after": 100,
            "limit": 10
        }))
        .unwrap();
        let req = tasks_tl.into_request();
        assert_eq!(req.name(), "harness_tasks");
        assert!(!req.is_mutating());

        // status
        let status: SessionArgs = serde_json::from_value(json!({
            "action": "status",
            "session": "s1"
        }))
        .unwrap();
        let req = status.into_request();
        assert_eq!(req.name(), "harness_checkpoint");
        assert!(!req.is_mutating());

        // handover
        let handover: SessionArgs = serde_json::from_value(json!({
            "action": "handover",
            "session": "s1",
            "kind": "summary"
        }))
        .unwrap();
        let req = handover.into_request();
        assert_eq!(req.name(), "harness_handover");
        assert!(!req.is_mutating());
    }

    #[test]
    fn session_args_rejects_invalid_actions_or_missing_session() {
        for value in [
            json!({}),
            json!({"action": "unknown", "session": "s"}),
            json!({"action": "checkpoint"}),
            json!({"action": "tasks"}),
            json!({"action": "handover"}),
            json!({"action": "status"}),
        ] {
            assert!(serde_json::from_value::<SessionArgs>(value).is_err());
        }
    }
}

#[cfg(test)]
mod doctor_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn doctor_tool_args_deserializes_and_dispatches_all_actions() {
        // env
        let env: DoctorToolArgs = serde_json::from_value(json!({
            "action": "env",
            "versions": true,
            "connection": true,
            "session": "s1"
        }))
        .unwrap();
        let req = env.into_request();
        assert_eq!(req.name(), "harness_doctor");
        assert!(!req.is_mutating());

        // syntax
        let syntax: DoctorToolArgs = serde_json::from_value(json!({
            "action": "syntax",
            "files": ["src/lib.rs"],
            "maxFiles": 5,
            "session": "s1"
        }))
        .unwrap();
        let req = syntax.into_request();
        assert_eq!(req.name(), "harness_doctor");
        assert!(!req.is_mutating());

        // logs
        let logs: DoctorToolArgs = serde_json::from_value(json!({
            "action": "logs",
            "limit": 10,
            "level": "error",
            "tool": "harness_read",
            "session": "s1"
        }))
        .unwrap();
        let req = logs.into_request();
        assert_eq!(req.name(), "harness_doctor");
        assert!(!req.is_mutating());
    }

    #[test]
    fn doctor_tool_args_rejects_unknown_action() {
        assert!(serde_json::from_value::<DoctorToolArgs>(json!({"action": "repair"})).is_err());
        assert!(serde_json::from_value::<DoctorToolArgs>(json!({})).is_ok());
    }
}
