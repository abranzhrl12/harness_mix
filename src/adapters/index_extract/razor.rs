use super::CONTRACT;
use super::csharp::blank_csharp;
use super::decode::decode;
use codegraph_native::buffers::NODE_KINDS;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};

/// Razor declarations plus native C# extraction of member blocks. Markup stays
/// outside the C# projection; its expressions are not guessed as C# declarations.
pub(super) fn extract_razor(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    super::super::razor_syntax::validate(path, source, context)?;
    let mut blocks = Vec::new();
    let mut symbols = super::super::csharp_syntax::collect_cshtml_blocks(
        source,
        &harness_domain::hash(source.as_bytes()),
        context,
        &mut blocks,
        false,
    )?;
    // The kernel supplies C# declarations, fields, properties, calls and type
    // references. Keep only Razor directives from the lightweight live scanner.
    symbols.retain(|s| s.kind.starts_with("razor_"));
    if symbols.len() >= 20_000 {
        return Err(("EXTRACTION_LIMIT", "Too many Razor declarations"));
    }
    let file_id = harness_domain::hash(format!("razor:file:{path}").as_bytes());
    let mut nodes = vec![json!({"id":file_id,"kind":"file","name":path,
        "qualifiedName":path,"filePath":path,"language":"razor",
        "startLine":1,"endLine":source.lines().count().max(1),"startColumn":0,"endColumn":0})];
    let ids = symbols
        .iter()
        .map(|s| {
            harness_domain::hash(
                format!("razor:{path}:{}:{}:{}", s.kind, s.container, s.bytes.start).as_bytes(),
            )
        })
        .collect::<Vec<_>>();
    let mut edges = Vec::new();
    let mut model_refs = Vec::new();
    let mut order = (0..symbols.len()).collect::<Vec<_>>();
    order.sort_by_key(|&i| {
        (
            symbols[i].bytes.start,
            std::cmp::Reverse(symbols[i].bytes.end),
        )
    });
    let mut parents: Vec<usize> = Vec::new();
    for index in order {
        let symbol = &symbols[index];
        context.check()?;
        let kind = match symbol.kind {
            "razor_model" => "type",
            "razor_section" => "module",
            "razor_helper" => "function",
            kind if NODE_KINDS.contains(&kind) => kind,
            _ => return Err(("EXTRACTION_DEFERRED", "Unsupported Razor declaration kind")),
        };
        let qualified = format!("{}.{}", symbol.container, symbol.name);
        let column = |at: usize| {
            source[..at]
                .rsplit('\n')
                .next()
                .unwrap_or("")
                .encode_utf16()
                .count()
        };
        nodes.push(json!({"id":ids[index],"kind":kind,"name":symbol.name,
            "qualifiedName":qualified,"filePath":path,"language":"razor",
            "startLine":symbol.start,"endLine":symbol.end,
            "startColumn":column(symbol.bytes.start),"endColumn":column(symbol.bytes.end),
            "signature":symbol.signature,"docstring":symbol.doc}));
        if symbol.kind == "razor_model" {
            model_refs.push(json!({"fromNodeId":ids[index],"referenceName":symbol.name,
                "referenceKind":"references","line":symbol.start,"column":column(symbol.bytes.start),"filePath":path}));
        }
        while parents.last().is_some_and(|&i| {
            symbols[i].bytes.end <= symbol.bytes.start || symbols[i].bytes.end < symbol.bytes.end
        }) {
            parents.pop();
        }
        let parent = parents.last().map_or(&file_id, |&i| &ids[i]);
        edges.push(
            json!({"source":parent,"target":ids[index],"kind":"contains","provenance":"heuristic"}),
        );
        parents.push(index);
    }
    let mut csharp = extract_razor_csharp(path, source, &blocks, &file_id, context)?;
    nodes.extend(csharp["nodes"].as_array_mut().ok_or(CONTRACT)?.drain(..));
    edges.extend(csharp["edges"].as_array_mut().ok_or(CONTRACT)?.drain(..));
    model_refs.extend(
        csharp["unresolvedReferences"]
            .as_array_mut()
            .ok_or(CONTRACT)?
            .drain(..),
    );
    model_refs.extend(extract_razor_render(path, source, &file_id, context)?);
    model_refs.extend(super::super::razor_mvc::collect(
        source, path, &file_id, &blocks, context,
    )?);
    if nodes.len() > 20_000 || edges.len() > 20_000 || model_refs.len() > 20_000 {
        return Err(("EXTRACTION_LIMIT", "Razor graph exceeds publication budget"));
    }
    Ok(json!({"nodes":nodes,"edges":edges,"unresolvedReferences":model_refs,"errors":[]}))
}

fn extract_razor_render(
    path: &str,
    source: &str,
    file_id: &str,
    context: &OperationContext,
) -> Result<Vec<Value>, Failure> {
    let projection = super::super::razor_render::project(source, "", context)?;
    let prepared = blank_csharp(&projection.text)?;
    let raw = match codegraph_native::extract(path, &prepared, "csharp") {
        Ok(raw) => raw,
        // Razor allows mixed markup inside render statements. Unsupported
        // render syntax leaves those refs pending without discarding members.
        Err(_) => return Ok(Vec::new()),
    };
    let graph = decode(&raw, path, "razor", context)?;
    let Some(render) = graph["nodes"]
        .as_array()
        .ok_or(CONTRACT)?
        .iter()
        .find(|n| n["qualifiedName"] == "__HarnessRazor::__HarnessRender")
        .and_then(|n| n["id"].as_str())
    else {
        return Ok(Vec::new());
    };
    let mut references = Vec::new();
    for reference in graph["unresolvedReferences"].as_array().ok_or(CONTRACT)? {
        context.check()?;
        if reference["fromNodeId"] != render || reference["referenceKind"] != "calls" {
            continue;
        }
        // MVC helpers are resolved by the MVC scanner with richer evidence.
        if reference["referenceName"]
            .as_str()
            .is_some_and(super::super::razor_mvc::is_mvc_helper_call)
        {
            continue;
        }
        let Some((line, col)) = projection.location(
            reference["line"].as_u64().ok_or(CONTRACT)? as usize,
            reference["column"].as_u64().ok_or(CONTRACT)? as usize,
        ) else {
            continue;
        };
        let mut reference = reference.clone();
        reference["fromNodeId"] = json!(file_id);
        reference["line"] = json!(line);
        reference["column"] = json!(col);
        references.push(reference);
    }
    Ok(references)
}

/// One kernel call per view. Blanking by UTF-16 units preserves native columns,
/// including astral text in markup. A single wrapper line is removed on output.
fn extract_razor_csharp(
    path: &str,
    source: &str,
    blocks: &[(std::ops::Range<usize>, &'static str)],
    file_id: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if blocks.is_empty() {
        return Ok(json!({"nodes":[],"edges":[],"unresolvedReferences":[]}));
    }
    let mut projection = String::from("class __HarnessRazor {\n");
    let mut block_index = 0;
    for (offset, ch) in source.char_indices() {
        if offset % 4096 == 0 {
            context.check()?;
        }
        while blocks
            .get(block_index)
            .is_some_and(|(r, _)| offset >= r.end)
        {
            block_index += 1;
        }
        if ch == '\n'
            || ch == '\r'
            || blocks
                .get(block_index)
                .is_some_and(|(r, _)| r.contains(&offset))
        {
            projection.push(ch);
        } else {
            projection.extend(std::iter::repeat_n(' ', ch.len_utf16()));
        }
    }
    projection.push_str("\n}");
    let prepared = blank_csharp(&projection)?;
    let raw = codegraph_native::extract(path, &prepared, "csharp").map_err(|_| {
        (
            "EXTRACTION_DEFERRED",
            "Razor C# block extraction failed; index unchanged",
        )
    })?;
    let mut graph = decode(&raw, path, "razor", context)?;
    let line_starts = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(offset, _)| offset + 1))
        .collect::<Vec<_>>();
    let mut ids = std::collections::BTreeMap::<String, String>::new();
    let mut nodes = Vec::new();
    for mut node in graph["nodes"].as_array_mut().ok_or(CONTRACT)?.drain(..) {
        context.check()?;
        let old = node["id"].as_str().ok_or(CONTRACT)?.to_owned();
        if node["kind"] == "file" || (node["kind"] == "class" && node["name"] == "__HarnessRazor") {
            ids.insert(old, file_id.to_owned());
            continue;
        }
        let qualified = node["qualifiedName"]
            .as_str()
            .ok_or(CONTRACT)?
            .strip_prefix("__HarnessRazor::")
            .ok_or(CONTRACT)?
            .replace("::", ".");
        for key in ["startLine", "endLine"] {
            let line = node[key].as_u64().ok_or(CONTRACT)?;
            node[key] = json!(line.checked_sub(1).filter(|n| *n > 0).ok_or(CONTRACT)?);
        }
        // Both directives share the generated class, but selectors must retain
        // the spelling exposed by the live Razor outline. Native columns use
        // UTF-16 units; convert back to a source byte before finding the block.
        let line_start = *line_starts
            .get(node["startLine"].as_u64().ok_or(CONTRACT)? as usize - 1)
            .ok_or(CONTRACT)?;
        let column = node["startColumn"].as_u64().ok_or(CONTRACT)? as usize;
        let mut units = 0;
        let mut offset = line_start;
        for ch in source[line_start..].chars() {
            if units >= column {
                break;
            }
            units += ch.len_utf16();
            offset += ch.len_utf8();
        }
        let block = blocks.partition_point(|(range, _)| range.start <= offset);
        let (_, directive) = block
            .checked_sub(1)
            .and_then(|i| blocks.get(i))
            .filter(|(range, _)| range.contains(&offset))
            .ok_or(CONTRACT)?;
        node["qualifiedName"] = json!(format!("@{directive}.{qualified}"));
        let id = harness_domain::hash(format!("razor:csharp:{path}:{old}").as_bytes());
        ids.insert(old, id.clone());
        node["id"] = json!(id);
        nodes.push(node);
    }
    let mut edges = Vec::new();
    for mut edge in graph["edges"].as_array_mut().ok_or(CONTRACT)?.drain(..) {
        for key in ["source", "target"] {
            edge[key] = json!(
                ids.get(edge[key].as_str().ok_or(CONTRACT)?)
                    .ok_or(CONTRACT)?
            );
        }
        if edge["source"] == edge["target"] {
            continue;
        }
        if let Some(line) = edge["line"].as_u64() {
            edge["line"] = json!(line.checked_sub(1).ok_or(CONTRACT)?);
        }
        edges.push(edge);
    }
    let mut references = Vec::new();
    for reference in graph["unresolvedReferences"]
        .as_array_mut()
        .ok_or(CONTRACT)?
    {
        context.check()?;
        reference["fromNodeId"] = json!(
            ids.get(reference["fromNodeId"].as_str().ok_or(CONTRACT)?)
                .ok_or(CONTRACT)?
        );
        reference["line"] = json!(
            reference["line"]
                .as_u64()
                .ok_or(CONTRACT)?
                .checked_sub(1)
                .ok_or(CONTRACT)?
        );
        // MVC helpers are resolved by the MVC scanner with richer evidence.
        if reference["referenceName"]
            .as_str()
            .is_some_and(super::super::razor_mvc::is_mvc_helper_call)
        {
            continue;
        }
        references.push(reference.clone());
    }
    Ok(json!({"nodes":nodes,"edges":edges,"unresolvedReferences":references}))
}
