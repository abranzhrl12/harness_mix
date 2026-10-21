use super::compat::strip_inocuous;
#[cfg(feature = "schema")]
use super::compat::{read_alias_schema, search_alias_schema};
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

impl TryFrom<ReadInput> for ReadArgs {
    type Error = String;
    fn try_from(input: ReadInput) -> Result<Self, Self::Error> {
        let end = match (input.end, input.lines) {
            (Some(end), None) => end,
            (None, Some(count)) if count > 0 && count <= 400 => input
                .start
                .checked_add(count - 1)
                .ok_or("read range overflow")?,
            (Some(_), Some(_)) => return Err("use either end or lines, not both".into()),
            _ => return Err("provide end or lines (1..400)".into()),
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
    /// Search file or directory; omitted, null, "." or "" selects the workspace root.
    /// path is an alias; supply only one selector, e.g. {"kind":"text","query":"Controller","scope":"Controllers"}.
    #[serde(alias = "path")]
    pub scope: Option<String>,
    pub limit: Option<usize>,
    /// Surrounding lines per text match (0..20), including exact source and file SHA-256.
    #[serde(rename = "contextLines", alias = "context_lines")]
    pub context_lines: Option<usize>,
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
            #[serde(rename = "contextLines", alias = "context_lines")]
            context_lines: Option<usize>,
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
            context_lines: input.context_lines,
            session: input.session,
        })
    }
}
