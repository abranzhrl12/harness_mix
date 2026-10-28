//! Cross-layer data lineage / field tracing across Form -> JSON -> C# Model -> SP Parameter -> SQL Column.
use super::model::{
    CrossLayerDataTrace, DataTraceLayer, DataTraceLayerKind, DataTraceMatch, Node,
};
use crate::adapters::project::Project;
use harness_domain::{
    Failure, contracts::ExploreArgs, encoding::decode_source, execution::OperationContext,
    results::prefix,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, Instant},
};

const MAX_TRACE_FILES: usize = 512;
const MAX_TRACE_BYTES: usize = 8 * 1024 * 1024;
const MAX_MATCHES_PER_LAYER: usize = 8;
const TRACE_TIME_BUDGET: Duration = Duration::from_millis(1500);

const CHAIN_HEADER: &str =
    "Form (.cshtml) → JSON/JS (.js) → C# Model/Controller (.cs) → SP Parameter (@...) → SQL Column (.sql)";

#[derive(Clone, Debug)]
struct IdentifierVariants {
    raw: String,
    clean: String,
    normalized: String,
    parameter: String,
    all_lower: Vec<String>,
}

/// Normalize an identifier by stripping `@` and common Hungarian type prefixes
/// (`t`, `i`, `n`, `b`, `f`, `d`) when followed by an uppercase ASCII letter.
pub(crate) fn normalize_field_identifier(raw: &str) -> (String, String, String) {
    let clean = raw.trim().trim_start_matches('@').to_owned();
    let bytes = clean.as_bytes();
    let normalized = if bytes.len() >= 2
        && matches!(bytes[0], b't' | b'i' | b'n' | b'b' | b'f' | b'd')
        && bytes[1].is_ascii_uppercase()
    {
        clean[1..].to_owned()
    } else {
        clean.clone()
    };
    let parameter = format!("@{clean}");
    (clean, normalized, parameter)
}

impl IdentifierVariants {
    fn from_identifier(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let (clean, normalized, parameter) = normalize_field_identifier(trimmed);
        if clean.len() < 2 {
            return None;
        }
        let mut variants = BTreeSet::new();
        variants.insert(clean.to_ascii_lowercase());
        if normalized.len() >= 2 {
            variants.insert(normalized.to_ascii_lowercase());
            for prefix in ['t', 'i', 'n', 'b', 'f', 'd'] {
                let mut chars = normalized.chars();
                if let Some(first) = chars.next() {
                    let candidate =
                        format!("{prefix}{}{}", first.to_ascii_uppercase(), chars.as_str());
                    variants.insert(candidate.to_ascii_lowercase());
                }
            }
        }
        Some(Self {
            raw: trimmed.to_owned(),
            clean,
            normalized,
            parameter,
            all_lower: variants.into_iter().collect(),
        })
    }

    fn matches_line(&self, line_lower: &str) -> Option<String> {
        for variant in &self.all_lower {
            if contains_identifier_token(line_lower, variant) {
                return Some(variant.clone());
            }
        }
        None
    }
}

fn is_ident_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn contains_identifier_token(haystack_lower: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() || haystack_lower.len() < needle_lower.len() {
        return false;
    }
    let bytes = haystack_lower.as_bytes();
    let mut start = 0;
    while let Some(pos) = haystack_lower[start..].find(needle_lower) {
        let idx = start + pos;
        let end = idx + needle_lower.len();
        let before_ok = idx == 0 || !is_ident_char(bytes[idx - 1]);
        let after_ok = end >= bytes.len() || !is_ident_char(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        start = idx + 1;
    }
    false
}

pub(super) fn trace_symbol_name(
    args: &ExploreArgs,
    resolved_symbol: &str,
    targets: &[Node],
) -> Option<String> {
    if !resolved_symbol.trim().is_empty() && !resolved_symbol.contains('/') {
        return Some(resolved_symbol.trim().to_owned());
    }
    if let Some(symbol) = args.symbol.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return Some(symbol.to_owned());
    }
    if let Some(first) = super::query_symbols(args).into_iter().next() {
        if !first.trim().is_empty() {
            return Some(first.trim().to_owned());
        }
    }
    targets
        .iter()
        .find(|node| node.kind != "file")
        .map(|node| node.name.clone())
}

pub(super) fn trace(
    project: &Project,
    args: &ExploreArgs,
    resolved_symbol: &str,
    targets: &[Node],
    graph_files: &BTreeSet<String>,
    context: &OperationContext,
) -> Result<Option<CrossLayerDataTrace>, Failure> {
    let Some(symbol_name) = trace_symbol_name(args, resolved_symbol, targets) else {
        return Ok(None);
    };
    let Some(variants) = IdentifierVariants::from_identifier(&symbol_name) else {
        return Ok(None);
    };
    let started = Instant::now();
    let mut layer_matches: BTreeMap<DataTraceLayerKind, Vec<DataTraceMatch>> = BTreeMap::new();
    let mut layer_omitted: BTreeMap<DataTraceLayerKind, usize> = BTreeMap::new();
    for kind in DataTraceLayerKind::ALL {
        layer_matches.insert(kind, Vec::new());
        layer_omitted.insert(kind, 0);
    }

    let mut seen_files = BTreeSet::new();
    let mut candidate_paths = Vec::new();
    if let Some(file_path) = args.file_path.as_deref() {
        let normalized = file_path
            .replace('\\', "/")
            .trim_start_matches("./")
            .trim_end_matches('/')
            .to_owned();
        if !normalized.is_empty() && is_traceable_path(&normalized) && seen_files.insert(normalized.clone()) {
            candidate_paths.push(normalized);
        }
    }
    for target in targets {
        if is_traceable_path(&target.file_path) && seen_files.insert(target.file_path.clone()) {
            candidate_paths.push(target.file_path.clone());
        }
    }
    for file in graph_files {
        if is_traceable_path(file) && seen_files.insert(file.clone()) {
            candidate_paths.push(file.clone());
        }
    }

    let mut truncated = false;
    let discovered = discover_workspace_files(
        project,
        &seen_files,
        MAX_TRACE_FILES.saturating_sub(candidate_paths.len()),
        context,
        &started,
        &mut truncated,
    )?;
    candidate_paths.extend(discovered);

    let mut bytes_read = 0usize;
    for path in candidate_paths {
        if context.check().is_err() || started.elapsed() >= TRACE_TIME_BUDGET {
            truncated = true;
            break;
        }
        if super::super::index_inventory::restricted(&path)
            || super::super::project::external_restricted(&path)
            || super::super::project::external_sensitive(&path)
        {
            continue;
        }
        let Ok(raw) = project.bytes_with_limit(&path, context, 512 * 1024) else {
            continue;
        };
        if bytes_read.saturating_add(raw.len()) > MAX_TRACE_BYTES {
            truncated = true;
            break;
        }
        bytes_read += raw.len();
        let Ok(source) = decode_source(&raw) else {
            continue;
        };
        scan_file_layers(
            &path,
            &source,
            &variants,
            &mut layer_matches,
            &mut layer_omitted,
            &mut truncated,
        );
    }

    Ok(Some(assemble_trace(
        &variants,
        layer_matches,
        layer_omitted,
        truncated,
    )))
}

fn is_traceable_path(path: &str) -> bool {
    let Some((_, ext)) = path.rsplit_once('.') else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "cshtml" | "html" | "htm" | "vue" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "cs" | "sql"
    )
}

fn discover_workspace_files(
    project: &Project,
    already_queued: &BTreeSet<String>,
    remaining_limit: usize,
    context: &OperationContext,
    started: &Instant,
    truncated: &mut bool,
) -> Result<Vec<String>, Failure> {
    if remaining_limit == 0 {
        return Ok(Vec::new());
    }
    let mut builder = ignore::WalkBuilder::new(&project.root);
    builder
        .follow_links(false)
        .hidden(false)
        .sort_by_file_path(|a, b| a.cmp(b))
        .filter_entry(|entry| {
            entry.depth() == 0
                || !super::super::index_inventory::excluded(&entry.file_name().to_string_lossy())
        });
    let ignore_path = project.root.join(".harnessignore");
    if ignore_path.exists() {
        let _ = builder.add_ignore(ignore_path);
    }
    let mut paths = Vec::new();
    let mut visited = 0usize;
    for entry in builder.build() {
        if context.check().is_err() || started.elapsed() >= TRACE_TIME_BUDGET {
            *truncated = true;
            break;
        }
        visited += 1;
        if visited > 15_000 {
            *truncated = true;
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(&project.root) else {
            continue;
        };
        let Some(relative_str) = relative.to_str() else {
            continue;
        };
        let normalized = relative_str.replace('\\', "/");
        if already_queued.contains(&normalized) || !is_traceable_path(&normalized) {
            continue;
        }
        if super::super::index_inventory::restricted(&normalized)
            || super::super::project::external_restricted(&normalized)
            || super::super::project::external_sensitive(&normalized)
        {
            continue;
        }
        if project.target_optional(&normalized).is_err() {
            continue;
        }
        paths.push(normalized);
        if paths.len() >= remaining_limit {
            *truncated = true;
            break;
        }
    }
    Ok(paths)
}

fn scan_file_layers(
    path: &str,
    source: &str,
    variants: &IdentifierVariants,
    layer_matches: &mut BTreeMap<DataTraceLayerKind, Vec<DataTraceMatch>>,
    layer_omitted: &mut BTreeMap<DataTraceLayerKind, usize>,
    truncated: &mut bool,
) {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut in_script_block = false;
    for (idx, raw_line) in source.lines().enumerate() {
        let line_num = idx + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();

        if matches!(ext.as_str(), "cshtml" | "html" | "htm" | "vue") {
            if lower.contains("<script") {
                in_script_block = true;
            }
            let current_in_script = in_script_block;
            if lower.contains("</script") {
                in_script_block = false;
            }
            let Some(matched_variant) = variants.matches_line(&lower) else {
                continue;
            };
            if current_in_script {
                if let Some(kind) = classify_json_js_line(&lower) {
                    push_match(
                        DataTraceLayerKind::Json,
                        DataTraceMatch {
                            file_path: path.to_owned(),
                            line: line_num,
                            kind: kind.to_owned(),
                            matched_token: matched_token_case(trimmed, &matched_variant, false),
                            snippet: clean_snippet(trimmed),
                        },
                        layer_matches,
                        layer_omitted,
                        truncated,
                    );
                    continue;
                }
            }
            if let Some(kind) = classify_form_line(&lower) {
                push_match(
                    DataTraceLayerKind::Form,
                    DataTraceMatch {
                        file_path: path.to_owned(),
                        line: line_num,
                        kind: kind.to_owned(),
                        matched_token: matched_token_case(trimmed, &matched_variant, false),
                        snippet: clean_snippet(trimmed),
                    },
                    layer_matches,
                    layer_omitted,
                    truncated,
                );
            } else if let Some(kind) = classify_json_js_line(&lower) {
                push_match(
                    DataTraceLayerKind::Json,
                    DataTraceMatch {
                        file_path: path.to_owned(),
                        line: line_num,
                        kind: kind.to_owned(),
                        matched_token: matched_token_case(trimmed, &matched_variant, false),
                        snippet: clean_snippet(trimmed),
                    },
                    layer_matches,
                    layer_omitted,
                    truncated,
                );
            } else {
                push_match(
                    DataTraceLayerKind::Form,
                    DataTraceMatch {
                        file_path: path.to_owned(),
                        line: line_num,
                        kind: "view_reference".to_owned(),
                        matched_token: matched_token_case(trimmed, &matched_variant, false),
                        snippet: clean_snippet(trimmed),
                    },
                    layer_matches,
                    layer_omitted,
                    truncated,
                );
            }
        } else if matches!(ext.as_str(), "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx") {
            let Some(matched_variant) = variants.matches_line(&lower) else {
                continue;
            };
            let kind = classify_json_js_line(&lower).unwrap_or("js_reference");
            push_match(
                DataTraceLayerKind::Json,
                DataTraceMatch {
                    file_path: path.to_owned(),
                    line: line_num,
                    kind: kind.to_owned(),
                    matched_token: matched_token_case(trimmed, &matched_variant, false),
                    snippet: clean_snippet(trimmed),
                },
                layer_matches,
                layer_omitted,
                truncated,
            );
        } else if ext == "cs" {
            let Some(matched_variant) = variants.matches_line(&lower) else {
                continue;
            };
            if let Some(sp_kind) = classify_csharp_sp_param_line(&lower, &matched_variant) {
                push_match(
                    DataTraceLayerKind::SpParameter,
                    DataTraceMatch {
                        file_path: path.to_owned(),
                        line: line_num,
                        kind: sp_kind.to_owned(),
                        matched_token: matched_token_case(trimmed, &matched_variant, true),
                        snippet: clean_snippet(trimmed),
                    },
                    layer_matches,
                    layer_omitted,
                    truncated,
                );
            }
            if let Some(cs_kind) = classify_csharp_model_line(&lower, &matched_variant) {
                push_match(
                    DataTraceLayerKind::CsharpModel,
                    DataTraceMatch {
                        file_path: path.to_owned(),
                        line: line_num,
                        kind: cs_kind.to_owned(),
                        matched_token: matched_token_case(trimmed, &matched_variant, false),
                        snippet: clean_snippet(trimmed),
                    },
                    layer_matches,
                    layer_omitted,
                    truncated,
                );
            }
        } else if ext == "sql" {
            let Some(matched_variant) = variants.matches_line(&lower) else {
                continue;
            };
            let has_at_param = contains_at_parameter(&lower, &matched_variant);
            let (sp_kind, col_kind) = classify_sql_line(&lower, &matched_variant, has_at_param);
            if let Some(kind) = sp_kind {
                push_match(
                    DataTraceLayerKind::SpParameter,
                    DataTraceMatch {
                        file_path: path.to_owned(),
                        line: line_num,
                        kind: kind.to_owned(),
                        matched_token: matched_token_case(trimmed, &matched_variant, true),
                        snippet: clean_snippet(trimmed),
                    },
                    layer_matches,
                    layer_omitted,
                    truncated,
                );
            }
            if let Some(kind) = col_kind {
                push_match(
                    DataTraceLayerKind::SqlColumn,
                    DataTraceMatch {
                        file_path: path.to_owned(),
                        line: line_num,
                        kind: kind.to_owned(),
                        matched_token: matched_token_case(trimmed, &matched_variant, false),
                        snippet: clean_snippet(trimmed),
                    },
                    layer_matches,
                    layer_omitted,
                    truncated,
                );
            }
        }
    }
}

fn classify_form_line(lower: &str) -> Option<&'static str> {
    if lower.contains("asp-for") {
        Some("asp_for")
    } else if lower.contains("v-model") {
        Some("v_model")
    } else if lower.contains("name=") || lower.contains("name =") {
        Some("form_input_name")
    } else if lower.contains("id=") || lower.contains("id =") {
        Some("form_element_id")
    } else if lower.contains("data-") {
        Some("data_attribute")
    } else if lower.contains("@model.")
        || lower.contains("html.textboxfor")
        || lower.contains("html.editorfor")
        || lower.contains("html.hiddenfor")
        || lower.contains("html.textareafor")
        || lower.contains("html.dropdownlistfor")
        || lower.contains("html.labelfor")
    {
        Some("razor_model_binding")
    } else if lower.contains("<input")
        || lower.contains("<select")
        || lower.contains("<textarea")
        || lower.contains("<label")
    {
        Some("form_control")
    } else {
        None
    }
}

fn classify_json_js_line(lower: &str) -> Option<&'static str> {
    if lower.contains("formdata") || lower.contains(".append(") {
        Some("form_data")
    } else if lower.contains("json.stringify")
        || lower.contains("$.ajax")
        || lower.contains("$.post")
        || lower.contains("$.get")
        || lower.contains("fetch(")
        || lower.contains("axios.")
    {
        Some("ajax_payload")
    } else if lower.contains("queryselector")
        || lower.contains("getelementbyid")
        || lower.contains("getelementsbyname")
        || lower.contains("$('#")
        || lower.contains("$(\"#")
        || lower.contains("[name=")
        || lower.contains(".val(")
        || lower.contains(".value")
    {
        Some("dom_selector")
    } else if lower.contains(':') || lower.contains('.') || lower.contains('[') {
        Some("json_property")
    } else {
        None
    }
}

fn contains_at_parameter(lower: &str, variant: &str) -> bool {
    let target = format!("@{variant}");
    let bytes = lower.as_bytes();
    let mut start = 0;
    while let Some(pos) = lower[start..].find(&target) {
        let idx = start + pos;
        let end = idx + target.len();
        let after_ok = end >= bytes.len() || !is_ident_char(bytes[end]);
        if after_ok {
            return true;
        }
        start = idx + 1;
    }
    false
}

fn classify_csharp_sp_param_line(lower: &str, variant: &str) -> Option<&'static str> {
    if lower.contains("sqlparameter") {
        Some("sql_parameter")
    } else if lower.contains("addwithvalue") {
        Some("add_with_value")
    } else if lower.contains("parameters.add") || lower.contains(".add(") && contains_at_parameter(lower, variant) {
        Some("parameter_add")
    } else if contains_at_parameter(lower, variant) {
        Some("sp_parameter_binding")
    } else {
        None
    }
}

fn classify_csharp_model_line(lower: &str, variant: &str) -> Option<&'static str> {
    if lower.contains("get;") || lower.contains("set;") {
        Some("csharp_property")
    } else if lower.contains("[jsonproperty")
        || lower.contains("[frombody")
        || lower.contains("[fromform")
        || lower.contains("[bindproperty")
    {
        Some("model_attribute")
    } else if (lower.contains("sqlparameter")
        || lower.contains("addwithvalue")
        || lower.contains("parameters.add"))
        && !lower.contains(&format!(".{variant}"))
        && !lower.contains(&format!("={variant}"))
        && !lower.contains(&format!(", {variant}"))
        && !lower.contains(&format!(",{variant}"))
    {
        // Pure `@param` literal string without referencing a C# property on the same line
        if count_variant_occurrences(lower, variant) <= 1 && contains_at_parameter(lower, variant)
        {
            return None;
        }
        Some("csharp_parameter_source")
    } else {
        Some("csharp_usage")
    }
}

fn count_variant_occurrences(lower: &str, variant: &str) -> usize {
    let bytes = lower.as_bytes();
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = lower[start..].find(variant) {
        let idx = start + pos;
        let end = idx + variant.len();
        let before_ok = idx == 0 || !is_ident_char(bytes[idx - 1]);
        let after_ok = end >= bytes.len() || !is_ident_char(bytes[end]);
        if before_ok && after_ok {
            count += 1;
        }
        start = idx + 1;
    }
    count
}

fn classify_sql_line(
    lower: &str,
    variant: &str,
    has_at_param: bool,
) -> (Option<&'static str>, Option<&'static str>) {
    let is_sql_data_type = [
        "varchar",
        "nvarchar",
        "int",
        "bigint",
        "smallint",
        "tinyint",
        "bit",
        "decimal",
        "numeric",
        "float",
        "datetime",
        "datetime2",
        "date",
        "char",
        "nchar",
        "text",
        "ntext",
        "uniqueidentifier",
        "money",
        "varbinary",
    ]
    .iter()
    .any(|ty| contains_identifier_token(lower, ty));

    let has_non_at_token = count_variant_occurrences(lower, variant)
        > usize::from(has_at_param);

    let sp_kind = if has_at_param {
        if is_sql_data_type {
            Some("sp_parameter_declaration")
        } else {
            Some("sp_parameter_reference")
        }
    } else {
        None
    };

    let col_kind = if has_non_at_token {
        if is_sql_data_type && !has_at_param {
            Some("table_column_definition")
        } else if lower.contains("insert ")
            || lower.contains("update ")
            || lower.contains("select ")
            || lower.contains("set ")
            || lower.contains("where ")
            || lower.contains("values")
            || lower.contains('=')
        {
            Some("sql_dml_column")
        } else {
            Some("sql_column_reference")
        }
    } else {
        None
    };

    (sp_kind, col_kind)
}

fn matched_token_case(line: &str, variant_lower: &str, prefer_at: bool) -> String {
    let lower = line.to_ascii_lowercase();
    if prefer_at {
        let at_target = format!("@{variant_lower}");
        if let Some(pos) = lower.find(&at_target) {
            if let Some(slice) = line.get(pos..pos + at_target.len()) {
                return slice.to_owned();
            }
        }
    }
    let bytes = lower.as_bytes();
    let mut start = 0;
    while let Some(pos) = lower[start..].find(variant_lower) {
        let idx = start + pos;
        let end = idx + variant_lower.len();
        let before_ok = idx == 0 || !is_ident_char(bytes[idx - 1]);
        let after_ok = end >= bytes.len() || !is_ident_char(bytes[end]);
        if before_ok && after_ok {
            if let Some(slice) = line.get(idx..end) {
                return slice.to_owned();
            }
        }
        start = idx + 1;
    }
    variant_lower.to_owned()
}

fn clean_snippet(trimmed: &str) -> String {
    let single_line = trimmed.replace(['\r', '\n', '\t'], " ");
    let collapsed = single_line.split_whitespace().collect::<Vec<_>>().join(" ");
    let clipped = prefix(&collapsed, 180);
    if clipped.len() < collapsed.len() {
        format!("{clipped}...")
    } else {
        collapsed
    }
}

fn push_match(
    layer: DataTraceLayerKind,
    item: DataTraceMatch,
    layer_matches: &mut BTreeMap<DataTraceLayerKind, Vec<DataTraceMatch>>,
    layer_omitted: &mut BTreeMap<DataTraceLayerKind, usize>,
    truncated: &mut bool,
) {
    let entries = layer_matches.entry(layer).or_default();
    if entries
        .iter()
        .any(|existing| existing.file_path == item.file_path && existing.line == item.line)
    {
        return;
    }
    if entries.len() >= MAX_MATCHES_PER_LAYER {
        *layer_omitted.entry(layer).or_default() += 1;
        *truncated = true;
        return;
    }
    entries.push(item);
}

fn assemble_trace(
    variants: &IdentifierVariants,
    mut layer_matches: BTreeMap<DataTraceLayerKind, Vec<DataTraceMatch>>,
    layer_omitted: BTreeMap<DataTraceLayerKind, usize>,
    truncated: bool,
) -> CrossLayerDataTrace {
    let mut layers = Vec::with_capacity(DataTraceLayerKind::ALL.len());
    let mut matched_layers = Vec::new();
    let mut missing_layers = Vec::new();
    let mut summary_parts = Vec::with_capacity(DataTraceLayerKind::ALL.len());

    for kind in DataTraceLayerKind::ALL {
        let mut matches = layer_matches.remove(&kind).unwrap_or_default();
        matches.sort_by(|a, b| {
            a.file_path
                .cmp(&b.file_path)
                .then_with(|| a.line.cmp(&b.line))
        });
        let omitted = layer_omitted.get(&kind).copied().unwrap_or(0);
        let matched = !matches.is_empty();
        if matched {
            matched_layers.push(kind);
            let first = &matches[0];
            summary_parts.push(format!(
                "{} [✓ {}:{}]",
                kind.label(),
                first.file_path,
                first.line
            ));
        } else {
            missing_layers.push(kind);
            summary_parts.push(format!("{} [✗ missing]", kind.label()));
        }
        layers.push(DataTraceLayer {
            layer: kind,
            label: kind.label().to_owned(),
            status: if matched {
                "matched".to_owned()
            } else {
                "missing".to_owned()
            },
            matched,
            matches,
            matches_omitted: omitted,
        });
    }

    let matched_layer_count = matched_layers.len();
    let total_layers = DataTraceLayerKind::ALL.len();
    let complete_chain = matched_layer_count == total_layers;

    CrossLayerDataTrace {
        identifier: variants.clean.clone(),
        normalized_identifier: variants.normalized.clone(),
        parameter_name: variants.parameter.clone(),
        chain: CHAIN_HEADER.to_owned(),
        summary_chain: summary_parts.join(" → "),
        complete_chain,
        matched_layer_count,
        total_layers,
        matched_layers,
        missing_layers,
        layers,
        truncated,
    }
}
