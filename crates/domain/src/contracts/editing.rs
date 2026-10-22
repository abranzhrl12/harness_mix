use super::compat::strip_inocuous;
#[cfg(feature = "schema")]
use super::compat::{replace_alias_schema, replace_chunk_alias_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct ReplaceArgs {
    pub path: String,
    /// Override UTF-8 BOM. Omitted preserves an existing BOM and enables it for Razor on Windows.
    pub bom: Option<bool>,
    /// Current source SHA-256, including matches[].hash from harness_search; baseHash is accepted as an alias. Supply only one.
    /// Optional on the wire; an omitted hash is bound to the source read during this edit.
    pub hash: String,
    /// Optional 1-based start line for oldText; disambiguates repeated text. Not valid with chunks.
    pub line: Option<usize>,
    pub chunks: Option<Vec<ReplaceChunk>>,
    pub old_text: String,
    pub new_text: String,
    /// Explicitly permit UTF-8 text when no syntax parser is available.
    pub allow_unvalidated_text: bool,
    /// When true, preview unified diff and syntax diagnostics without publishing.
    pub preview: bool,
    pub session: Option<String>,
}

impl<'de> serde::Deserialize<'de> for ReplaceArgs {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut value = Value::deserialize(deserializer)?;
        strip_inocuous(&mut value);
        normalize_replace_keys(&mut value).map_err(serde::de::Error::custom)?;
        let input: ReplaceInput =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Self::try_from(input).map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for ReplaceArgs {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ReplaceArgs".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        ReplaceInput::json_schema(generator)
    }
}

/// Accept casing and underscore variants without accepting unknown keys or
/// silently choosing between two spellings of the same argument.
fn normalize_replace_keys(value: &mut Value) -> Result<(), String> {
    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };
    let mut normalized = serde_json::Map::new();
    for (key, mut value) in std::mem::take(object) {
        let folded = key.replace('_', "").to_ascii_lowercase();
        let canonical = match folded.as_str() {
            "path" | "targetpath" => "path",
            "bom" => "bom",
            "hash" | "basehash" => "hash",
            "chunks" => "chunks",
            "oldtext" => "oldText",
            "newtext" => "newText",
            "allowunvalidatedtext" => "allowUnvalidatedText",
            "preview" => "preview",
            "session" | "sessionid" => "session",
            "line" => "line",
            _ => &key,
        }
        .to_string();
        if canonical == "chunks" {
            if let Some(chunks) = value.as_array_mut() {
                for chunk in chunks {
                    normalize_replace_keys(chunk)?;
                }
            }
        }
        if normalized.insert(canonical.clone(), value).is_some() {
            return Err(format!("duplicate field `{canonical}`"));
        }
    }
    *object = normalized;
    Ok(())
}

impl Default for ReplaceArgs {
    fn default() -> Self {
        Self {
            path: String::new(),
            bom: None,
            hash: String::new(),
            line: None,
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

/// Permanently delete one regular file, only when authorized by the user.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DeleteArgs {
    pub path: String,
    /// Optional SHA-256 of the file. A mismatch prevents deletion.
    #[serde(default, rename = "expectedHash")]
    pub expected_hash: Option<String>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ProposalAction {
    Prepare,
}

/// Persist a candidate for an existing file without publishing it.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ProposalArgs {
    pub action: ProposalAction,
    pub path: String,
    /// Complete proposed UTF-8 content. No source file is written.
    pub content: String,
    #[serde(default, rename = "expectedHash")]
    pub expected_hash: Option<String>,
    /// Omitted preserves the existing UTF-8 BOM policy.
    pub bom: Option<bool>,
    #[serde(default, rename = "allowUnvalidatedText")]
    pub allow_unvalidated_text: bool,
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
        if let Some(line) = self.line {
            map.serialize_entry("line", &line)?;
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
#[cfg_attr(feature = "schema", schemars(transform = replace_chunk_alias_schema))]
pub struct ReplaceChunk {
    /// Exact source to replace; matches[].context.text from harness_search can be reused when it contains sufficient, untruncated context.
    #[serde(rename = "oldText", alias = "old_text")]
    pub old_text: String,
    #[serde(rename = "newText", alias = "new_text")]
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
    /// Optional full-file SHA-256; reuse matches[].hash from harness_search. baseHash is an accepted alias; supply only one name. Omission binds the edit to its internal source read; a supplied hash detects concurrent changes.
    #[serde(default, alias = "baseHash")]
    hash: String,
    /// Optional 1-based start line for oldText. Omit hash to bind to the internally read source.
    line: Option<usize>,
    chunks: Option<Vec<ReplaceChunk>>,
    /// Exact source to replace; reuse matches[].context.text from harness_search when the context is sufficient and untruncated. Preserve its original line endings; do not use line-numbered display text.
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
            line: input.line,
            chunks: input.chunks,
            old_text,
            new_text,
            allow_unvalidated_text: input.allow_unvalidated_text,
            preview: input.preview,
            session: input.session,
        })
    }
}
