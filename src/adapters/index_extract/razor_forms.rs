use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast::Expression};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType};
use serde_json::{Value, json};
use std::ops::Range;

const CONTRACT: Failure = ("EXTRACTION_CONTRACT", "Invalid Razor form graph");

fn ident(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn tag_end(text: &str, start: usize, name: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let next = *bytes.get(start + 1 + name.len())?;
    if !next.is_ascii_whitespace() && next != b'>' && next != b'/' {
        return None;
    }
    let mut quote = None;
    for (i, byte) in bytes.iter().enumerate().skip(start + 1 + name.len()) {
        if let Some(q) = quote {
            if *byte == q {
                quote = None;
            }
        } else if *byte == b'\'' || *byte == b'"' {
            quote = Some(*byte);
        } else if *byte == b'>' {
            return Some(i);
        }
    }
    None
}

pub(super) fn link(
    path: &str,
    source: &str,
    graph: &mut Value,
    context: &OperationContext,
    regions: &[Range<usize>],
) -> Result<(), Failure> {
    let markup = &source[..regions.first().map_or(source.len(), |r| r.start)];
    if markup.contains('@') || markup.contains("<!--") || markup.matches("<form").count() != 1 {
        return Ok(());
    }
    let Some(start) = markup.find("<form") else {
        return Ok(());
    };
    let Some(open_end) = tag_end(markup, start, "form") else {
        return Ok(());
    };
    let Some(end) = markup[open_end + 1..]
        .find("</form>")
        .map(|n| open_end + 1 + n)
    else {
        return Ok(());
    };
    let Some(form_name) =
        super::http_razor::attribute(&markup[start + 5..open_end], "id").filter(|v| ident(v))
    else {
        return Ok(());
    };
    let body = &markup[open_end + 1..end];
    let Some(button) = body.find("<button") else {
        return Ok(());
    };
    let Some(button_end) = tag_end(body, button, "button") else {
        return Ok(());
    };
    let Some(handler) = super::http_razor::attribute(&body[button + 7..button_end], "onclick")
    else {
        return Ok(());
    };
    let handler = handler.trim().strip_suffix("()").unwrap_or("");
    if !ident(handler) {
        return Ok(());
    }
    let mut fields = Vec::new();
    let mut at = 0;
    while let Some(relative) = body[at..].find("<input") {
        let begin = at + relative;
        let Some(close) = tag_end(body, begin, "input") else {
            return Ok(());
        };
        if let Some(name) =
            super::http_razor::attribute(&body[begin + 6..close], "name").filter(|v| ident(v))
        {
            fields.push((name, open_end + 1 + begin, open_end + 2 + close));
        }
        at = close + 1;
    }
    if fields.is_empty()
        || fields
            .iter()
            .any(|(name, _, _)| fields.iter().filter(|(n, _, _)| n == name).count() != 1)
    {
        return Ok(());
    }
    let nodes = graph["nodes"].as_array().ok_or(CONTRACT)?;
    let handlers = nodes
        .iter()
        .filter(|n| n["kind"] == "function" && n["name"] == handler)
        .collect::<Vec<_>>();
    if handlers.len() != 1 {
        return Ok(());
    }
    let handler_node = handlers[0];
    let handler_id = handler_node["id"].as_str().ok_or(CONTRACT)?.to_owned();
    let mut used = std::collections::BTreeSet::new();
    for region in regions {
        context.check()?;
        let script = &source[region.clone()];
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, script, SourceType::default()).parse();
        if parsed.diagnostics.has_errors() {
            continue;
        }
        let semantic = SemanticBuilder::new()
            .with_build_nodes(true)
            .build(&parsed.program)
            .semantic;
        // Any local binding of these globals makes static identification ambiguous.
        if semantic.nodes().iter().any(|n| matches!(n.kind(), AstKind::BindingIdentifier(id) if ["document", "JSON", "fetch"].contains(&id.name.as_str()))) {
            continue;
        }
        for node in semantic.nodes().iter() {
            let AstKind::CallExpression(call) = node.kind() else {
                continue;
            };
            let Expression::StaticMemberExpression(member) = &call.callee else {
                continue;
            };
            if member.property.name != "querySelector"
                || !matches!(&member.object, Expression::Identifier(id) if id.name == "document")
            {
                continue;
            }
            let Some(Expression::StringLiteral(value)) =
                call.arguments.first().and_then(|a| a.as_expression())
            else {
                continue;
            };
            let Some(selector) = value.value.as_str() else {
                continue;
            };
            let Some(name) = selector
                .strip_prefix("[name=")
                .and_then(|v| v.strip_suffix(']'))
            else {
                continue;
            };
            if !ident(name) {
                continue;
            }
            let ancestors = semantic.nodes().ancestors(node.id()).collect::<Vec<_>>();
            let value_read = ancestors.iter().any(|a| matches!(a.kind(), AstKind::StaticMemberExpression(m) if m.property.name == "value" && matches!(&m.object, Expression::CallExpression(inner) if inner.span == call.span)));
            let stringify = ancestors.iter().find_map(|a| match a.kind() {
                AstKind::CallExpression(c) if matches!(&c.callee, Expression::StaticMemberExpression(m) if m.property.name == "stringify" && matches!(&m.object, Expression::Identifier(id) if id.name == "JSON")) => Some(c),
                _ => None,
            });
            let fetch = ancestors.iter().find_map(|a| match a.kind() {
                AstKind::CallExpression(c) if matches!(&c.callee, Expression::Identifier(id) if id.name == "fetch") => Some(c),
                _ => None,
            });
            let payload = match (stringify, fetch) {
                (Some(json_call), Some(fetch_call)) => {
                    match fetch_call.arguments.get(1).and_then(|a| a.as_expression()) {
                        Some(Expression::ObjectExpression(options)) => options.properties.iter().any(|entry| match entry {
                            oxc_ast::ast::ObjectPropertyKind::ObjectProperty(prop)
                                if !prop.computed && matches!(&prop.key, oxc_ast::ast::PropertyKey::StaticIdentifier(id) if id.name == "body") =>
                                prop.value.span().start <= json_call.span.start && prop.value.span().end >= json_call.span.end,
                            _ => false,
                        }),
                        _ => false,
                    }
                }
                _ => false,
            };
            let (line, col) =
                super::framework::position(source, region.start + call.span().start as usize)?;
            let sl = handler_node["startLine"].as_u64().unwrap_or(0);
            let el = handler_node["endLine"].as_u64().unwrap_or(0);
            let sc = handler_node["startColumn"].as_u64().unwrap_or(0);
            let ec = handler_node["endColumn"].as_u64().unwrap_or(0);
            let within = ((line as u64) > sl || (line as u64) == sl && (col as u64) >= sc)
                && ((line as u64) < el || (line as u64) == el && (col as u64) < ec);
            if value_read && payload && within && fields.iter().any(|(field, _, _)| field == name) {
                used.insert(name.to_owned());
            }
        }
    }
    let file_id = nodes
        .iter()
        .find(|n| n["kind"] == "file")
        .and_then(|n| n["id"].as_str())
        .ok_or(CONTRACT)?
        .to_owned();
    let form_id = super::framework::id("razor", path, "form", start);
    let (sl, sc) = super::framework::position(source, start)?;
    let (el, ec) = super::framework::position(source, end + 7)?;
    let mut new_nodes = vec![
        json!({"id":form_id,"kind":"form","name":form_name,"qualifiedName":format!("{path}::form@{start}"),"filePath":path,"language":"razor","startLine":sl,"startColumn":sc,"endLine":el,"endColumn":ec}),
    ];
    let mut edges = vec![
        json!({"source":file_id,"target":form_id,"kind":"contains","provenance":"heuristic","metadata":{"resolvedBy":"razor-static-form-v1","confidence":0.9}}),
        json!({"source":form_id,"target":handler_id,"kind":"calls","provenance":"heuristic","metadata":{"resolvedBy":"razor-inline-handler-v1","confidence":0.8}}),
    ];
    for (name, begin, end) in fields {
        let id = super::framework::id("razor", path, "field", begin);
        let (sl, sc) = super::framework::position(source, begin)?;
        let (el, ec) = super::framework::position(source, end)?;
        new_nodes.push(json!({"id":id,"kind":"property","name":name,"qualifiedName":format!("{path}::form-field@{begin}"),"filePath":path,"language":"razor","startLine":sl,"startColumn":sc,"endLine":el,"endColumn":ec}));
        edges.push(json!({"source":form_id,"target":id,"kind":"contains","provenance":"heuristic","metadata":{"resolvedBy":"razor-static-input-v1","confidence":0.9}}));
        if used.contains(&name) {
            edges.push(json!({"source":handler_id,"target":id,"kind":"references","provenance":"heuristic","metadata":{"resolvedBy":"razor-json-selector-v1","confidence":0.7}}));
        }
    }
    graph["nodes"]
        .as_array_mut()
        .ok_or(CONTRACT)?
        .extend(new_nodes);
    graph["edges"].as_array_mut().ok_or(CONTRACT)?.extend(edges);
    Ok(())
}
