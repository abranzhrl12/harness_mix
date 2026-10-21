use super::super::{CONTRACT, framework};
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::ops::Range;

const LIMIT: Failure = (
    "EXTRACTION_LIMIT",
    "Angular structural extraction limit exceeded",
);
const INVALID: Failure = (
    "EXTRACTION_DEFERRED",
    "Template structure could not be projected safely",
);
const MAX_ITEMS: usize = 5000;

#[derive(Clone)]
struct Tag<'a> {
    name: &'a str,
    start: usize,
    end: usize,
    closing: bool,
    self_closing: bool,
    attrs: Range<usize>,
}

fn parse_tag(source: &str, start: usize) -> Option<Tag<'_>> {
    let b = source.as_bytes();
    if b.get(start) != Some(&b'<') {
        return None;
    }
    let mut p = start + 1;
    let closing = b.get(p) == Some(&b'/');
    if closing {
        p += 1;
    }
    let ns = p;
    while b
        .get(p)
        .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b':' | b'_' | b'.'))
    {
        p += 1;
    }
    if p == ns {
        return None;
    }
    let ne = p;
    let attrs = p;
    let mut quote = None;
    while p < b.len() {
        if let Some(q) = quote {
            if b[p] == q {
                quote = None;
            }
        } else if matches!(b[p], b'\'' | b'"') {
            quote = Some(b[p]);
        } else if b[p] == b'>' {
            let self_closing = !closing
                && b[attrs..p]
                    .iter()
                    .rposition(|c| !c.is_ascii_whitespace())
                    .is_some_and(|i| b[attrs + i] == b'/');
            return Some(Tag {
                name: &source[ns..ne],
                start,
                end: p + 1,
                closing,
                self_closing,
                attrs: attrs..p,
            });
        }
        p += 1;
    }
    None
}

fn attributes(source: &str, tag: &Tag<'_>) -> Vec<(Range<usize>, Range<usize>, String)> {
    let b = source.as_bytes();
    let mut p = tag.attrs.start;
    let mut out = Vec::new();
    while p < tag.attrs.end {
        while p < tag.attrs.end && (b[p].is_ascii_whitespace() || b[p] == b'/') {
            p += 1;
        }
        let start = p;
        while p < tag.attrs.end && !b[p].is_ascii_whitespace() && !matches!(b[p], b'=' | b'/') {
            p += 1;
        }
        if start == p {
            p += 1;
            continue;
        }
        let name_end = p;
        while p < tag.attrs.end && b[p].is_ascii_whitespace() {
            p += 1;
        }
        if b.get(p) != Some(&b'=') {
            out.push((start..name_end, start..name_end, String::new()));
            continue;
        }
        p += 1;
        while p < tag.attrs.end && b[p].is_ascii_whitespace() {
            p += 1;
        }
        if p == tag.attrs.end {
            break;
        }
        let (vs, ve) = if matches!(b[p], b'\'' | b'"') {
            let q = b[p];
            p += 1;
            let s = p;
            while p < tag.attrs.end && b[p] != q {
                p += 1;
            }
            let e = p;
            if p < tag.attrs.end {
                p += 1;
            }
            (s, e)
        } else {
            let s = p;
            while p < tag.attrs.end && !b[p].is_ascii_whitespace() {
                p += 1;
            }
            (s, p)
        };
        out.push((start..p, start..name_end, source[vs..ve].to_owned()));
    }
    out
}

fn is_void(name: &str) -> bool {
    [
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr",
    ]
    .contains(&name.to_ascii_lowercase().as_str())
}

fn raw_text_end(source: &str, from: usize, name: &str) -> Option<usize> {
    let prefix = format!("</{name}");
    source[from..].match_indices('<').find_map(|(at, _)| {
        let at = from + at;
        if !source
            .as_bytes()
            .get(at..at + prefix.len())?
            .eq_ignore_ascii_case(prefix.as_bytes())
        {
            return None;
        }
        let tag = parse_tag(source, at)?;
        (tag.closing && tag.name.eq_ignore_ascii_case(name)).then_some(tag.end)
    })
}

fn flow_end(source: &str, start: usize) -> Option<usize> {
    let rest = source.get(start + 1..)?;
    if source.as_bytes().get(start) != Some(&b'@') {
        return None;
    }
    let name = [
        "if", "for", "switch", "defer", "else", "empty", "case", "default",
    ]
    .into_iter()
    .find(|n| {
        rest.starts_with(n)
            && !rest
                .as_bytes()
                .get(n.len())
                .is_some_and(u8::is_ascii_alphanumeric)
    })?;
    let mut p = start + 1 + name.len();
    while source
        .as_bytes()
        .get(p)
        .is_some_and(u8::is_ascii_whitespace)
    {
        p += 1;
    }
    if !matches!(name, "else" | "empty" | "default") && source.as_bytes().get(p) != Some(&b'(') {
        return None;
    }
    if source.as_bytes().get(p) == Some(&b'(') {
        let mut depth = 0;
        let mut quote = None;
        let mut escaped = false;
        while p < source.len() {
            if p.saturating_sub(start) > 16_384 {
                return None;
            }
            let b = source.as_bytes()[p];
            if let Some(q) = quote {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == q {
                    quote = None;
                }
            } else if matches!(b, b'\'' | b'"' | b'`') {
                quote = Some(b);
            } else if b == b'(' {
                depth += 1;
                if depth > 128 {
                    return None;
                }
            } else if b == b')' {
                depth -= 1;
                if depth == 0 {
                    p += 1;
                    break;
                }
            }
            p += 1;
        }
        if depth != 0 {
            return None;
        }
        while source
            .as_bytes()
            .get(p)
            .is_some_and(u8::is_ascii_whitespace)
        {
            p += 1;
        }
    }
    if matches!(name, "if" | "for" | "switch" | "defer" | "else")
        && source.as_bytes().get(p) != Some(&b'{')
    {
        return None;
    }
    if matches!(name, "empty" | "default") && source.as_bytes().get(p) == Some(&b'{') {
        p += 1;
    }
    Some(p)
}

fn add_fact(
    path: &str,
    positions: &framework::Positions<'_>,
    offset: usize,
    range: Range<usize>,
    name: &str,
    kind: &str,
    signature: String,
    parent: &str,
    language: &str,
    nodes: &mut Vec<Value>,
    edges: &mut Vec<Value>,
) -> Result<(), Failure> {
    let start = offset + range.start;
    let end = offset + range.end;
    let (line, col) = positions.position(start)?;
    let (el, ec) = positions.position(end)?;
    let id = framework::id("angular", path, kind, start);
    nodes.push(json!({"id":id,"kind":"property","name":name,"qualifiedName":format!("{path}::template::{kind}@{start}"),"filePath":path,"language":language,"startLine":line,"endLine":el,"startColumn":col,"endColumn":ec,"signature":signature}));
    edges.push(json!({"source":parent,"target":id,"kind":"contains","provenance":"heuristic"}));
    Ok(())
}

fn template_graph(
    path: &str,
    source: &str,
    template: &str,
    offset: usize,
    owner: &str,
    language: &str,
    context: &OperationContext,
) -> Result<(Vec<Value>, Vec<Value>, Vec<String>), Failure> {
    if template.len() > 1024 * 1024 {
        return Err(LIMIT);
    }
    let positions = framework::Positions::new(source);
    let (sl, sc) = positions.position(offset)?;
    let (el, ec) = positions.position(offset + template.len())?;
    let root = framework::id("angular", path, "template", offset);
    let mut nodes = vec![
        json!({"id":root,"kind":"module","name":"template","qualifiedName":format!("{path}::template@{offset}"),"filePath":path,"language":language,"startLine":sl,"endLine":el,"startColumn":sc,"endColumn":ec,"signature":"template"}),
    ];
    let mut edges =
        vec![json!({"source":owner,"target":root,"kind":"contains","provenance":"heuristic"})];
    let mut notices = Vec::new();
    let mut stack: Vec<(String, String)> = Vec::new();
    let mut cursor = 0;
    let mut items = 1;
    while cursor < template.len() {
        context.check()?;
        if items > MAX_ITEMS {
            return Err(LIMIT);
        }
        if template[cursor..].starts_with("<!--") {
            cursor = template[cursor + 4..]
                .find("-->")
                .map(|n| cursor + 4 + n + 3)
                .ok_or(INVALID)?;
            continue;
        }
        // One forward scan avoids repeatedly searching the entire remaining
        // template for absent interpolation/control-flow markers.
        let next = template[cursor..].char_indices().find_map(|(n, ch)| {
            let at = cursor + n;
            match ch {
                '<' => Some((at, 0)),
                '{' if template[at..].starts_with("{{") => Some((at, 1)),
                '@' => Some((at, 2)),
                _ => None,
            }
        });
        let Some((at, kind)) = next else {
            break;
        };
        if kind == 0 && template[at..].starts_with("<!--") {
            cursor = template[at + 4..]
                .find("-->")
                .map(|n| at + 4 + n + 3)
                .ok_or(INVALID)?;
            continue;
        }
        if kind == 1 {
            let end = template[at + 2..]
                .find("}}")
                .map(|n| at + 2 + n + 2)
                .ok_or(INVALID)?;
            let body = template[at + 2..end - 2].trim();
            if !body.is_empty() {
                let parent = stack
                    .last()
                    .map(|(_, id)| id.as_str())
                    .unwrap_or(root.as_str());
                add_fact(
                    path,
                    &positions,
                    offset,
                    at..end,
                    "interpolation",
                    "interpolation",
                    format!("{{{{ {body} }}}}"),
                    parent,
                    language,
                    &mut nodes,
                    &mut edges,
                )?;
                items += 1;
            }
            cursor = end;
            continue;
        }
        if kind == 2 {
            let Some(end) = flow_end(template, at) else {
                cursor = at + 1;
                continue;
            };
            let signature = template[at..end].trim().to_owned();
            let name = signature
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '@')
                .next()
                .unwrap_or("@flow")
                .to_owned();
            let parent = stack
                .last()
                .map(|(_, id)| id.as_str())
                .unwrap_or(root.as_str());
            add_fact(
                path,
                &positions,
                offset,
                at..end,
                &name,
                "control-flow",
                signature,
                parent,
                language,
                &mut nodes,
                &mut edges,
            )?;
            items += 1;
            cursor = end;
            continue;
        }
        let Some(parsed) = parse_tag(template, at) else {
            cursor = at + 1;
            continue;
        };
        if parsed.name.starts_with('!') {
            cursor = parsed.end;
            continue;
        }
        if parsed.closing {
            if let Some((name, _)) = stack.last() {
                if name.eq_ignore_ascii_case(parsed.name) {
                    stack.pop();
                } else {
                    notices
                        .push("Mismatched closing HTML tag retained as partial structure".into());
                }
            }
            cursor = parsed.end;
            continue;
        }
        let abs = offset + parsed.start;
        let (line, col) = positions.position(abs)?;
        let (end_line, end_col) = positions.position(offset + parsed.end)?;
        let id = framework::id("angular", path, "html-element", abs);
        let qname = format!("{path}::template::element[{}]@{abs}", parsed.name);
        nodes.push(json!({"id":id,"kind":"variable","name":parsed.name,"qualifiedName":qname,"filePath":path,"language":language,"startLine":line,"endLine":end_line,"startColumn":col,"endColumn":end_col,"signature":template[parsed.start..parsed.end]}));
        let parent = stack
            .last()
            .map(|(_, p)| p.as_str())
            .unwrap_or(root.as_str());
        edges.push(json!({"source":parent,"target":id,"kind":"contains","provenance":"heuristic"}));
        items += 1;
        for (range, name_range, value) in attributes(template, &parsed) {
            let name = &template[name_range];
            let fact = if name.starts_with('*') || name.starts_with('@') {
                Some("structural-binding")
            } else if name.starts_with("[(") || name.starts_with('[') || name.starts_with("bind-") {
                Some("property-binding")
            } else if name.starts_with('(') || name.starts_with("on-") {
                Some("event-binding")
            } else if name.starts_with('#') || name.starts_with("ref-") {
                Some("template-reference")
            } else if name.starts_with("let-") {
                Some("template-variable")
            } else {
                None
            };
            if let Some(kind) = fact {
                add_fact(
                    path,
                    &positions,
                    offset,
                    range,
                    name,
                    kind,
                    format!("{name}={value}"),
                    &id,
                    language,
                    &mut nodes,
                    &mut edges,
                )?;
                items += 1;
            }
        }
        if items > MAX_ITEMS {
            return Err(LIMIT);
        }
        if !parsed.self_closing
            && (parsed.name.eq_ignore_ascii_case("script")
                || parsed.name.eq_ignore_ascii_case("style"))
        {
            // Script/style bodies are raw text in HTML, not template markup.
            cursor = raw_text_end(template, parsed.end, parsed.name).unwrap_or_else(|| {
                notices.push("Unclosed raw-text HTML block skipped".into());
                template.len()
            });
            continue;
        }
        if !parsed.self_closing && !is_void(parsed.name) {
            stack.push((parsed.name.to_owned(), id));
        }
        cursor = parsed.end;
    }
    if items > MAX_ITEMS {
        return Err(LIMIT);
    }
    if !stack.is_empty() {
        notices.push("Unclosed HTML tags retained as partial structure".into());
    }
    Ok((nodes, edges, notices))
}

pub(super) fn inline(
    path: &str,
    source: &str,
    text: &str,
    offset: usize,
    owner: &str,
    language: &str,
    context: &OperationContext,
) -> Result<(Vec<Value>, Vec<Value>, Vec<String>), Failure> {
    template_graph(path, source, text, offset, owner, language, context)
}

pub(in crate::adapters::index_extract) fn html(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if source.len() > 1024 * 1024 {
        return Err(LIMIT);
    }
    let file = format!("file:{path}");
    let (nodes, edges, notices) = template_graph(path, source, source, 0, &file, "html", context)?;
    let mut graph = json!({"nodes":[framework::file_node(path,source,"html")],"edges":edges,"unresolvedReferences":[],"errors":[],
        "coverage":{"parser":"html-structural-subset-v1","limitations":["Generic HTML structure only; no Angular context, directive semantics, validation or template type checking"],"notices":notices}});
    graph["nodes"].as_array_mut().ok_or(CONTRACT)?.extend(nodes);
    Ok(graph)
}
