use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::sync::OnceLock;

pub fn language(path: &str) -> Option<&'static str> {
    match path.rsplit_once('.')?.1.to_ascii_lowercase().as_str() {
        "rs" => Some("rust"),
        "cs" => Some("csharp"),
        "cshtml" => Some("razor"),
        "vue" => Some("vue"),
        "html" | "htm" => Some("html"),
        "ts" | "mts" | "cts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        "jsx" => Some("jsx"),
        "scss" => Some("scss"),
        _ => None,
    }
}

pub fn extract(path: &str, source: &str, context: &OperationContext) -> Result<Value, Failure> {
    super::take_diagnostic();
    context.check()?;
    if source.len() > super::super::index_sync::MAX_SOURCE_BYTES {
        return Err((
            "EXTRACTION_LIMIT",
            "Source exceeds native extraction size budget",
        ));
    }
    let lang = language(path).ok_or((
        "SYNC_LANGUAGE",
        "Source language has no native CodeGraph extractor",
    ))?;
    if lang == "razor" {
        let mut graph = super::razor::extract_razor(path, source, context)?;
        super::http_razor::enrich(path, source, &mut graph, context)?;
        return Ok(graph);
    }
    if lang == "vue" {
        let mut graph = super::vue::extract(path, source, context)?;
        super::http_frontend::enrich(path, source, &mut graph, context)?;
        return Ok(graph);
    }
    if lang == "html" {
        let mut graph = super::angular::html(path, source, context)?;
        super::http_razor::enrich(path, source, &mut graph, context)?;
        return Ok(graph);
    }
    if lang == "scss" {
        return super::scss::extract(path, source, context);
    }
    let prepared = if lang == "csharp" {
        super::csharp::blank_csharp(source)?
    } else {
        source.to_owned()
    };
    let raw = codegraph_native::extract(path, &prepared, lang).map_err(|error| {
        let mut diagnostic = serde_json::json!({"file":path,"language":lang,"stage":"native_extract","message":error});
        if lang == "rust" {
            let parsed = ra_ap_syntax::SourceFile::parse(source, ra_ap_syntax::Edition::Edition2024);
            let errors: Vec<_> = parsed.errors().iter().map(|error| {
                let offset = usize::from(error.range().start());
                let prefix = &source[..offset.min(source.len())];
                let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
                let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
                serde_json::json!({"line":line,"column":column,"message":error.to_string()})
            }).collect();
            diagnostic["errors"] = serde_json::json!(errors);
        }
        super::set_diagnostic(diagnostic);
        (
            "EXTRACTION_DEFERRED",
            "Native extractor deferred this source; no Node fallback was executed",
        )
    })?;
    context.check()?;
    let mut graph = super::decode::decode(&raw, path, lang, context)?;
    if lang == "rust" {
        super::rust::enrich(path, source, &mut graph, context)?;
    }
    if lang == "csharp" {
        collect_sql_command_refs(source, &mut graph, context)?;
        super::http_backend::enrich(path, source, &mut graph, context)?;
    }
    if matches!(lang, "typescript" | "tsx" | "javascript" | "jsx") {
        super::angular::enrich(path, source, &mut graph, context)?;
        super::vue_router::enrich(path, source, &mut graph, context)?;
        super::http_frontend::enrich(path, source, &mut graph, context)?;
    }
    Ok(graph)
}

/// Add references for statically named SqlCommand constructors. Dynamic and
/// escaped C# strings remain unresolved by design.
fn collect_sql_command_refs(
    source: &str,
    graph: &mut Value,
    context: &OperationContext,
) -> Result<(), Failure> {
    static COMMAND: OnceLock<regex::Regex> = OnceLock::new();
    static NAME: OnceLock<regex::Regex> = OnceLock::new();
    let command = COMMAND.get_or_init(|| regex::Regex::new(r"\bnew\s+SqlCommand\s*\(").unwrap());
    let name = NAME.get_or_init(|| regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_$]*(?:\.[A-Za-z_][A-Za-z0-9_$]*)?$").unwrap());
    let masked = mask_csharp_non_code(source);
    let masked = std::str::from_utf8(&masked).map_err(|_| super::CONTRACT)?;
    let file_id = graph["nodes"]
        .as_array()
        .ok_or(super::CONTRACT)?
        .iter()
        .find(|node| node["kind"] == "file")
        .and_then(|node| node["id"].as_str())
        .ok_or(super::CONTRACT)?
        .to_owned();
    let nodes = graph["nodes"].as_array().ok_or(super::CONTRACT)?;
    let mut found = Vec::new();
    for matched in command.find_iter(masked) {
        context.check()?;
        let opening = matched.end().checked_sub(1).ok_or(super::CONTRACT)?;
        let mut tail = source.get(opening + 1..).ok_or(super::CONTRACT)?;
        tail = tail.trim_start();
        let Some(literal) = tail.strip_prefix('"') else { continue };
        let Some(end) = literal.find('"') else { continue };
        let value = &literal[..end];
        if value.contains('\\') || !name.is_match(value) {
            continue;
        }
        let after = literal[end + 1..].trim_start();
        if !after.starts_with(',') && !after.starts_with(')') {
            continue;
        }
        let at = matched.start();
        let prefix = &source[..at];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = prefix
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .trim_end_matches('\r')
            .encode_utf16()
            .count();
        let owner = nodes
            .iter()
            .filter(|node| node["kind"] == "method")
            .filter(|node| {
                let start = node["startLine"].as_u64().unwrap_or(0) as usize;
                let end = node["endLine"].as_u64().unwrap_or(0) as usize;
                let starts_before_call = line > start
                    || node["startColumn"]
                        .as_u64()
                        .is_none_or(|column_start| column >= column_start as usize);
                let ends_after_call = line < end
                    || node["endColumn"]
                        .as_u64()
                        .is_none_or(|column_end| column < column_end as usize);
                start <= line && line <= end && starts_before_call && ends_after_call
            })
            .min_by_key(|node| {
                (
                    node["endLine"]
                        .as_u64()
                        .unwrap_or(0)
                        .saturating_sub(node["startLine"].as_u64().unwrap_or(0)),
                    node["endColumn"]
                        .as_u64()
                        .unwrap_or(u64::MAX)
                        .saturating_sub(node["startColumn"].as_u64().unwrap_or(0)),
                )
            })
            .and_then(|node| node["id"].as_str())
            .unwrap_or(&file_id);
        found.push(json!({
            "fromNodeId":owner,
            "referenceName":value,
            "referenceKind":"stored_procedure_call",
            "line":line,
            "column":column,
            "filePath":graph["nodes"].as_array().and_then(|nodes| nodes.iter().find(|node| node["kind"] == "file")).and_then(|node| node.get("filePath"))
        }));
    }
    graph["unresolvedReferences"]
        .as_array_mut()
        .ok_or(super::CONTRACT)?
        .extend(found);
    Ok(())
}

/// Blank C# comments, strings and chars so constructor-shaped text inside them
/// cannot produce references. Length and newline positions remain unchanged.
fn mask_csharp_non_code(source: &str) -> Vec<u8> {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut at = 0;
    while at < bytes.len() {
        let end = if bytes[at] == b'/' && bytes.get(at + 1) == Some(&b'/') {
            let start = at;
            at += 2;
            while at < bytes.len() && bytes[at] != b'\n' { at += 1; }
            Some(start..at)
        } else if bytes[at] == b'/' && bytes.get(at + 1) == Some(&b'*') {
            let start = at;
            at += 2;
            while at + 1 < bytes.len() && !(bytes[at] == b'*' && bytes[at + 1] == b'/') { at += 1; }
            at = (at + 2).min(bytes.len());
            Some(start..at)
        } else if matches!(bytes[at], b'"' | b'\'') {
            let quote = bytes[at];
            let start = at;
            let quote_count = if quote == b'"' {
                bytes[at..].iter().take_while(|byte| **byte == quote).count()
            } else {
                1
            };
            if quote == b'"' && quote_count >= 3 {
                // C# raw strings close with a quote run at least as long as
                // the opening delimiter. Quotes in the content may be shorter.
                at += quote_count;
                while at < bytes.len() {
                    if bytes[at] == quote {
                        let run = bytes[at..].iter().take_while(|byte| **byte == quote).count();
                        if run >= quote_count {
                            at += run;
                            break;
                        }
                        at += run;
                    } else {
                        at += 1;
                    }
                }
            } else {
                let verbatim = quote == b'"'
                    && (bytes.get(at.wrapping_sub(1)) == Some(&b'@')
                        || at >= 2 && bytes[at - 2..at] == [b'@', b'$']);
                at += 1;
                while at < bytes.len() {
                    if verbatim && bytes[at] == quote && bytes.get(at + 1) == Some(&quote) {
                        at += 2;
                        continue;
                    }
                    if !verbatim && bytes[at] == b'\\' {
                        at = (at + 2).min(bytes.len());
                        continue;
                    }
                    if bytes[at] == quote {
                        at += 1;
                        break;
                    }
                    at += 1;
                }
            }
            Some(start..at)
        } else {
            at += 1;
            None
        };
        if let Some(range) = end {
            for byte in &mut masked[range] {
                if !matches!(*byte, b'\n' | b'\r') { *byte = b' '; }
            }
        }
    }
    masked
}

#[cfg(test)]
#[path = "orchestration_tests.rs"]
mod sql_command_tests;
