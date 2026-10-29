//! Static HTTP route extraction from inline HTML/Razor scripts.
//! Razor Url.Action accepts only two simple literal arguments and a conventional /Controller/Action path.
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::ops::Range;

const CONTRACT: Failure = ("EXTRACTION_CONTRACT", "Invalid Razor HTTP extraction graph");
const LIMIT: Failure = ("EXTRACTION_LIMIT", "Razor HTTP extraction limit exceeded");

#[derive(Clone, Copy)]
struct Unit {
    ch: char,
    original: usize,
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub(super) fn attribute(attrs: &str, wanted: &str) -> Option<String> {
    let bytes = attrs.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        while at < bytes.len() && (bytes[at].is_ascii_whitespace() || bytes[at] == b'/') {
            at += 1;
        }
        let start = at;
        while at < bytes.len()
            && !bytes[at].is_ascii_whitespace()
            && bytes[at] != b'='
            && bytes[at] != b'/'
        {
            at += 1;
        }
        if start == at {
            at += 1;
            continue;
        }
        let name = attrs.get(start..at)?;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if bytes.get(at) != Some(&b'=') {
            continue;
        }
        at += 1;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let value_start;
        let value_end;
        if matches!(bytes.get(at), Some(b'"' | b'\'')) {
            let quote = bytes[at];
            at += 1;
            value_start = at;
            while at < bytes.len() && bytes[at] != quote {
                at += 1;
            }
            value_end = at;
            at = (at + 1).min(bytes.len());
        } else {
            value_start = at;
            while at < bytes.len() && !bytes[at].is_ascii_whitespace() && bytes[at] != b'>' {
                at += 1;
            }
            value_end = at;
        }
        if name.eq_ignore_ascii_case(wanted) {
            return Some(attrs.get(value_start..value_end)?.to_owned());
        }
    }
    None
}

fn scripts(
    source: &str,
    razor: bool,
    context: &OperationContext,
) -> Result<Vec<Range<usize>>, Failure> {
    if source.len() > 4 * 1024 * 1024 {
        return Err(LIMIT);
    }
    let bytes = source.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        context.check()?;
        if bytes.get(at..at + 2) == Some(b"@*") && razor {
            at += 2;
            while at + 1 < bytes.len() && bytes.get(at..at + 2) != Some(b"*@") {
                at += 1;
            }
            at = (at + 2).min(bytes.len());
            continue;
        }
        if bytes.get(at..at + 4) == Some(b"<!--") {
            at += 4;
            while at + 2 < bytes.len() && bytes.get(at..at + 3) != Some(b"-->") {
                at += 1;
            }
            at = (at + 3).min(bytes.len());
            continue;
        }
        if bytes.get(at) != Some(&b'<')
            || !bytes
                .get(at + 1..at + 7)
                .is_some_and(|s| s.eq_ignore_ascii_case(b"script"))
        {
            at += 1;
            continue;
        }
        if bytes
            .get(at + 7)
            .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'>' && *b != b'/')
        {
            at += 1;
            continue;
        }
        let open = at;
        let mut end_open = at + 7;
        let mut quote = None;
        while end_open < bytes.len() {
            match (quote, bytes[end_open]) {
                (Some(q), b) if q == b => quote = None,
                (None, b'\'' | b'"') => quote = Some(bytes[end_open]),
                (None, b'>') => break,
                _ => {}
            }
            end_open += 1;
        }
        if end_open == bytes.len() {
            break;
        }
        let attrs = &source[open + 7..end_open];
        let script_type = attribute(attrs, "type");
        let supported_type = script_type.as_deref().is_none_or(|kind| {
            matches!(
                kind.trim().to_ascii_lowercase().as_str(),
                "text/javascript"
                    | "application/javascript"
                    | "module"
                    | "text/ecmascript"
                    | "application/ecmascript"
            )
        });
        let external = attribute(attrs, "src").is_some();
        let body_start = end_open + 1;
        let mut close = body_start;
        while close < bytes.len() {
            if bytes[close] == b'<'
                && bytes.get(close + 1) == Some(&b'/')
                && bytes
                    .get(close + 2..close + 8)
                    .is_some_and(|s| s.eq_ignore_ascii_case(b"script"))
                && bytes
                    .get(close + 8)
                    .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>')
            {
                break;
            }
            close += 1;
        }
        if supported_type && !external && body_start <= close {
            found.push(body_start..close);
            if found.len() > 256 {
                return Err(LIMIT);
            }
        }
        if close == bytes.len() {
            break;
        }
        at = close + 2;
    }
    Ok(found)
}

fn string_arg(s: &str) -> Option<&str> {
    let s = s.trim();
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains(['\\', '\n', '\r']) && identifier(inner)).then_some(inner)
}

fn helper_end(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 1usize;
    let mut quote = None;
    let mut escaped = false;
    let mut i = open + 1;
    while i < bytes.len() && i - open < 4096 {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if bytes[i] == b'\\' {
                escaped = true;
            } else if bytes[i] == active {
                quote = None;
            }
        } else {
            match bytes[i] {
                b'"' | b'\'' => quote = Some(bytes[i]),
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn url_action(source: &str, at: usize, end: usize) -> Option<String> {
    let call = source.get(at..end)?;
    let open = call.find('(')?;
    let close = call.rfind(')')?;
    let args = &call[open + 1..close];
    let mut quoted = false;
    let mut depth = 0usize;
    let mut comma = None;
    for (i, ch) in args.char_indices() {
        match ch {
            '"' if i == 0 || args.as_bytes()[i - 1] != b'\\' => quoted = !quoted,
            '(' | '{' | '[' if !quoted => depth += 1,
            ')' | '}' | ']' if !quoted => depth = depth.checked_sub(1)?,
            ',' if !quoted && depth == 0 => {
                comma = Some(i);
                break;
            }
            _ => {}
        }
    }
    let comma = comma?;
    let action = string_arg(&args[..comma])?;
    let controller = string_arg(&args[comma + 1..])?;
    Some(format!("/{controller}/{action}"))
}

fn transformed(
    source: &str,
    regions: &[Range<usize>],
    razor: bool,
    context: &OperationContext,
) -> Result<(String, Vec<usize>), Failure> {
    let mut units = Vec::<Unit>::new();
    let mut cursor = 0usize;
    for region in regions {
        context.check()?;
        if region.start < cursor
            || region.end > source.len()
            || source.get(region.clone()).is_none()
        {
            return Err(CONTRACT);
        }
        for (off, ch) in source[cursor..region.start].char_indices() {
            units.push(Unit {
                ch: if matches!(ch, '\r' | '\n') { ch } else { ' ' },
                original: cursor + off,
            });
        }
        let mut pos = region.start;
        while pos < region.end {
            if razor && source[pos..region.end].starts_with("@*") {
                let comment_end = source[pos + 2..region.end]
                    .find("*@")
                    .map(|end| pos + 2 + end + 2)
                    .unwrap_or(region.end);
                for (offset, ch) in source[pos..comment_end].char_indices() {
                    units.push(Unit {
                        ch: if matches!(ch, '\r' | '\n') { ch } else { ' ' },
                        original: pos + offset,
                    });
                }
                pos = comment_end;
                continue;
            }
            if razor
                && source
                    .get(pos..pos + 12)
                    .is_some_and(|value| value.eq_ignore_ascii_case("@Url.Action("))
            {
                let open = pos + 11;
                if let Some(end) = helper_end(source, open) {
                    if let Some(url) = url_action(source, pos, end) {
                        for ch in url.chars() {
                            units.push(Unit { ch, original: pos });
                        }
                        pos = end;
                        continue;
                    }
                }
            }
            let ch = source[pos..].chars().next().ok_or(CONTRACT)?;
            units.push(Unit { ch, original: pos });
            pos += ch.len_utf8();
        }
        cursor = region.end;
    }
    for (off, ch) in source[cursor..].char_indices() {
        units.push(Unit {
            ch: if matches!(ch, '\r' | '\n') { ch } else { ' ' },
            original: cursor + off,
        });
    }
    let mut output = String::with_capacity(source.len());
    let mut map = Vec::with_capacity(units.len() + 1);
    for unit in units {
        // Preserve UTF-16 columns where non-script Unicode is masked.
        if unit.ch == ' ' && unit.original < source.len() {
            let original_ch = source[unit.original..].chars().next().ok_or(CONTRACT)?;
            let repeat = if original_ch != '\r' && original_ch != '\n' {
                original_ch.len_utf16()
            } else {
                1
            };
            for _ in 0..repeat {
                output.push(' ');
                map.push(unit.original);
            }
        } else {
            output.push(unit.ch);
            map.extend(std::iter::repeat_n(unit.original, unit.ch.len_utf8()));
        }
    }
    map.push(source.len());
    Ok((output, map))
}

fn output_offset(source: &str, line: usize, column: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    let start = if line == 1 {
        0
    } else {
        source.match_indices('\n').nth(line - 2)?.0 + 1
    };
    let tail = source.get(start..)?;
    let row_end = tail.find('\n').map_or(source.len(), |i| start + i);
    let row = &source[start..row_end];
    let mut units = 0;
    for (i, ch) in row.char_indices() {
        if units == column {
            return Some(start + i);
        }
        units += ch.len_utf16();
        if units > column {
            return None;
        }
    }
    (units == column).then_some(row_end)
}

fn source_position(source: &str, offset: usize) -> Option<(usize, usize)> {
    let prefix = source.get(..offset)?;
    Some((
        prefix.bytes().filter(|b| *b == b'\n').count() + 1,
        prefix.rsplit('\n').next()?.encode_utf16().count(),
    ))
}

fn map_position(
    output: &str,
    original: &str,
    map: &[usize],
    line: usize,
    column: usize,
) -> Option<(usize, usize)> {
    let offset = output_offset(output, line, column)?;
    let original_offset = *map.get(offset)?;
    source_position(original, original_offset)
}

pub(super) fn enrich(
    path: &str,
    source: &str,
    graph: &mut Value,
    context: &OperationContext,
) -> Result<(), Failure> {
    context.check()?;
    let razor = path.to_ascii_lowercase().ends_with(".cshtml");
    let regions = scripts(source, razor, context)?;
    if regions.is_empty() {
        return Ok(());
    }
    if enrich_script(path, source, graph, context, razor, &regions)? {
        if razor {
            super::razor_forms::link(path, source, graph, context, &regions)?;
        }
        return Ok(());
    }
    if source.len().saturating_mul(regions.len()) > 32 * 1024 * 1024 {
        return Err(LIMIT);
    }
    let mut incomplete = false;
    for region in regions {
        if !enrich_script(path, source, graph, context, razor, &[region])? {
            incomplete = true;
        }
    }
    if incomplete {
        graph["coverage"] = json!({"complete":false,"status":"partial","reason":"invalid_inline_script"});
    }
    Ok(())
}

fn enrich_script(
    path: &str,
    source: &str,
    graph: &mut Value,
    context: &OperationContext,
    razor: bool,
    regions: &[Range<usize>],
) -> Result<bool, Failure> {
    let (projected, map) = transformed(source, regions, razor, context)?;
    let raw = match codegraph_native::extract(path, &projected, "javascript") {
        Ok(raw) => raw,
        Err(_) => return Ok(false),
    };
    let mut js_graph = super::decode::decode(&raw, path, "javascript", context)?;
    let reference_start = js_graph["unresolvedReferences"]
        .as_array()
        .ok_or(CONTRACT)?
        .len();
    super::http_frontend::enrich(path, &projected, &mut js_graph, context)?;

    let js_nodes = js_graph["nodes"].as_array().ok_or(CONTRACT)?.clone();
    let js_file_ids = js_nodes
        .iter()
        .filter(|node| node["kind"] == "file")
        .filter_map(|node| node["id"].as_str().map(str::to_owned))
        .collect::<std::collections::BTreeSet<_>>();
    let file_id = graph["nodes"]
        .as_array()
        .ok_or(CONTRACT)?
        .iter()
        .find(|node| node["kind"] == "file")
        .and_then(|node| node["id"].as_str())
        .ok_or(CONTRACT)?
        .to_owned();
    let mut caller_nodes = Vec::new();
    for mut node in js_nodes {
        if !matches!(node["kind"].as_str(), Some("function" | "method")) {
            continue;
        }
        let (Some(start_line), Some(start_col), Some(end_line), Some(end_col)) = (
            node["startLine"].as_u64(),
            node["startColumn"].as_u64(),
            node["endLine"].as_u64(),
            node["endColumn"].as_u64(),
        ) else {
            continue;
        };
        let Some((sl, sc)) = map_position(
            &projected,
            source,
            &map,
            start_line as usize,
            start_col as usize,
        ) else {
            continue;
        };
        let Some((el, ec)) = map_position(
            &projected,
            source,
            &map,
            end_line as usize,
            end_col as usize,
        ) else {
            continue;
        };
        node["startLine"] = json!(sl);
        node["startColumn"] = json!(sc);
        node["endLine"] = json!(el);
        node["endColumn"] = json!(ec);
        node["filePath"] = json!(path);
        node["language"] = json!("razor");
        caller_nodes.push(node);
    }
    let caller_ids = caller_nodes
        .iter()
        .filter_map(|node| node["id"].as_str().map(str::to_owned))
        .collect::<std::collections::BTreeSet<_>>();
    let edges = js_graph["edges"]
        .as_array()
        .ok_or(CONTRACT)?
        .iter()
        .filter(|edge| caller_ids.contains(edge["target"].as_str().unwrap_or("")))
        .map(|edge| {
            let mut edge = edge.clone();
            if js_file_ids.contains(edge["source"].as_str().unwrap_or(""))
                || !caller_ids.contains(edge["source"].as_str().unwrap_or(""))
            {
                edge["source"] = json!(file_id);
            }
            edge
        })
        .collect::<Vec<_>>();
    graph["nodes"]
        .as_array_mut()
        .ok_or(CONTRACT)?
        .extend(caller_nodes);
    graph["edges"].as_array_mut().ok_or(CONTRACT)?.extend(edges);

    let mut references = Vec::new();
    for mut reference in js_graph["unresolvedReferences"]
        .as_array()
        .ok_or(CONTRACT)?
        .iter()
        .skip(reference_start)
        .cloned()
    {
        if reference["referenceKind"] != "http_route" {
            continue;
        }
        let Some((line, column)) = (|| {
            let line = reference["line"].as_u64()? as usize;
            let column = reference["column"].as_u64()? as usize;
            map_position(&projected, source, &map, line, column)
        })() else {
            continue;
        };
        let url = serde_json::from_str::<Value>(reference["referenceName"].as_str().unwrap_or(""))
            .ok()
            .and_then(|route| route["url"].as_str().map(str::to_owned))
            .unwrap_or_default();
        // An unsupported/dynamic Razor helper must not become a guessed URL.
        if url.to_ascii_lowercase().contains("@url.action") {
            continue;
        }
        reference["line"] = json!(line);
        reference["column"] = json!(column);
        if js_file_ids.contains(reference["fromNodeId"].as_str().unwrap_or("")) {
            reference["fromNodeId"] = json!(file_id);
        }
        reference["filePath"] = json!(path);
        references.push(reference);
    }
    graph["unresolvedReferences"]
        .as_array_mut()
        .ok_or(CONTRACT)?
        .extend(references);
    Ok(true)
}

#[cfg(test)]
#[path = "http_razor_tests.rs"]
mod tests;
