use super::compat::strip_inocuous;
#[cfg(feature = "schema")]
use super::compat::{read_alias_schema, search_alias_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct BootstrapArgs {
    /// Opt in to a compact initial catalog; safety notices remain inline.
    #[serde(default)]
    pub compact: bool,
    /// Reuse data.session from bootstrap. sessionId is accepted for compatibility.
    #[serde(alias = "sessionId")]
    pub session: Option<String>,
    /// Legacy input accepted but ignored; project instructions live in skills.
    #[cfg_attr(feature = "schema", schemars(skip))]
    #[serde(rename = "includeRules")]
    pub include_rules: Option<bool>,
}

impl<'de> Deserialize<'de> for BootstrapArgs {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            #[serde(default)]
            compact: bool,
            #[serde(alias = "sessionId")]
            session: Option<String>,
            #[serde(rename = "includeRules")]
            include_rules: Option<bool>,
            #[serde(
                default,
                rename = "action",
                deserialize_with = "reject_bootstrap_action"
            )]
            _action: Option<Value>,
        }

        let input = Input::deserialize(deserializer)?;
        Ok(Self {
            compact: input.compact,
            session: input.session,
            include_rules: input.include_rules,
        })
    }
}

fn reject_bootstrap_action<'de, D: serde::Deserializer<'de>>(
    _deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Err(serde::de::Error::custom(
        "bootstrap does not accept action; use harness_doctor with action=env, syntax, logs, process, or health",
    ))
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
    /// Remove only empty envelope metadata and the public request ID; defaults to false.
    pub compact: bool,
    /// Add display-only absolute line numbers while preserving `text`; defaults to false.
    #[serde(rename = "lineNumbers")]
    pub line_numbers: bool,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("oneOf" = [
    {"required":["end"],"not":{"required":["lines"]}},
    {"required":["lines"],"not":{"required":["end"]}},
    {"not":{"anyOf":[{"required":["end"]},{"required":["lines"]}]}}
])))]
#[cfg_attr(feature = "schema", schemars(transform = read_alias_schema))]
struct ReadInput {
    #[serde(rename = "externalScope")]
    external_scope: Option<String>,
    /// File path; file and target_path are accepted aliases. Supply only one.
    #[serde(alias = "target_path", alias = "file")]
    path: String,
    #[serde(default = "default_read_start")]
    start: usize,
    #[serde(default)]
    end: Option<usize>,
    /// Number of lines starting at start; mutually exclusive with end.
    #[serde(default)]
    lines: Option<usize>,
    /// Compact envelope presentation; disabled by default.
    #[serde(default)]
    compact: bool,
    /// Add display-only absolute line numbers; disabled by default.
    #[serde(default, rename = "lineNumbers")]
    line_numbers: bool,
    #[serde(alias = "sessionId")]
    session: Option<String>,
}

fn default_read_start() -> usize {
    1
}

impl TryFrom<ReadInput> for ReadArgs {
    type Error = String;
    fn try_from(input: ReadInput) -> Result<Self, Self::Error> {
        if input.start == 0 {
            return Err("start must be at least 1".into());
        }
        let end = match (input.end, input.lines) {
            (Some(end), None) => end,
            (None, Some(count)) if count > 0 && count <= 400 => input
                .start
                .checked_add(count - 1)
                .ok_or("read range overflow")?,
            (None, None) => input.start.checked_add(119).ok_or("read range overflow")?,
            (Some(_), Some(_)) => return Err("use either end or lines, not both".into()),
            _ => return Err("provide end or lines (1..400)".into()),
        };
        if end < input.start {
            return Err("end must be greater than or equal to start".into());
        }
        if end - input.start >= 400 {
            return Err("read range cannot exceed 400 lines".into());
        }
        Ok(Self {
            external_scope: input.external_scope,
            path: input.path,
            start: input.start,
            end,
            compact: input.compact,
            line_numbers: input.line_numbers,
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
    #[serde(default = "default_search_kind")]
    #[cfg_attr(feature = "schema", schemars(default = "default_search_kind"))]
    pub kind: Kind,
    #[serde(default, alias = "pattern")]
    pub query: String,
    #[serde(rename = "minLines")]
    pub min_lines: Option<usize>,
    /// File patterns; includes is accepted as an alias. Supply only one.
    #[serde(alias = "includes")]
    pub globs: Option<Vec<String>>,
    /// Search file or directory; omitted, null, "." or "" selects the workspace root.
    /// path is an alias; both selectors are allowed only when equivalent.
    #[serde(alias = "path")]
    pub scope: Option<String>,
    pub limit: Option<usize>,
    /// Surrounding lines per text match (0..20). Each match includes hash (full-file SHA-256) and context: {start, end, text}, with 1-based inclusive line bounds and exact source preserving original line endings. For a sufficient, untruncated context, pass matches[].file as harness_replace.path, matches[].context.text as oldText, and matches[].hash as hash without a redundant read; the hash detects concurrent changes.
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
            #[serde(default = "default_search_kind")]
            kind: Kind,
            #[serde(default, alias = "pattern")]
            query: String,
            #[serde(rename = "minLines")]
            min_lines: Option<usize>,
            #[serde(alias = "includes")]
            globs: Option<Vec<String>>,
            scope: Option<String>,
            path: Option<String>,
            limit: Option<usize>,
            #[serde(rename = "contextLines", alias = "context_lines")]
            context_lines: Option<usize>,
            session: Option<String>,
        }
        let mut value = Value::deserialize(deserializer)?;
        strip_inocuous(&mut value);
        let input = Input::deserialize(value).map_err(|error| {
            serde::de::Error::custom(format!(
                "{error}; scope and path may both be supplied when equivalent; if they differ, provide only one, e.g. {{\"kind\":\"text\",\"query\":\"Controller\",\"scope\":\"Controllers\"}}"
            ))
        })?;
        let scope = match (input.scope, input.path) {
            (Some(scope), Some(path))
                if normalize_search_scope(&scope) != normalize_search_scope(&path) =>
            {
                return Err(serde::de::Error::custom(
                    "scope and path refer to different locations; provide only one or make them equivalent, e.g. {\"kind\":\"text\",\"query\":\"Controller\",\"scope\":\"Controllers\"}",
                ));
            }
            (Some(scope), _) => Some(scope),
            (None, path) => path,
        };
        Ok(Self {
            external_scope: input.external_scope,
            kind: input.kind,
            query: input.query,
            min_lines: input.min_lines,
            globs: input.globs,
            scope,
            limit: input.limit,
            context_lines: input.context_lines,
            session: input.session,
        })
    }
}

fn default_search_kind() -> Kind {
    Kind::Text
}

fn normalize_search_scope(scope: &str) -> String {
    let normalized = scope.trim().replace('\\', "/");
    let drive_root = normalized.as_bytes().get(1) == Some(&b':')
        && normalized
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        && normalized.as_bytes().get(2) == Some(&b'/');
    let root = if drive_root {
        &normalized[..3]
    } else if normalized.starts_with("//") {
        "//"
    } else if normalized.starts_with('/') {
        "/"
    } else {
        ""
    };
    let tail = if drive_root {
        &normalized[3..]
    } else {
        &normalized
    };
    let components = tail
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>();
    if components.is_empty() {
        if root.is_empty() {
            ".".to_owned()
        } else {
            root.to_owned()
        }
    } else {
        format!("{root}{}", components.join("/"))
    }
}
