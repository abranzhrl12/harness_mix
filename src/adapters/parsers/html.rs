//! Native, read-only structural navigation. Never serialize a DOM back over source.
use super::model::Symbol;
use harness_domain::{Failure, execution::OperationContext, hash, results::prefix};
use serde_json::{Value, json};
mod selectors;

const LIMIT: Failure = (
    "PARSER_LIMIT",
    "HTML structure exceeds source, node or attribute budget",
);

pub(in crate::adapters) fn collect(
    source: &str,
    digest: &str,
    context: &OperationContext,
    selector: Option<&str>,
    include_structure: bool,
) -> Result<(Vec<Symbol>, Option<Value>), Failure> {
    context.check()?;
    if source.len() > 2 * 1024 * 1024 {
        return Err(LIMIT);
    }
    let projected = project_raw_text(source, context)?;
    let dom = tl::parse(&projected, tl::ParserOptions::default())
        .map_err(|_| ("HTML_PARSE", "HTML parser could not construct a document"))?;
    context.check()?;
    if dom.nodes().len() > 20000 {
        return Err(LIMIT);
    }
    let parser = dom.parser();
    let selected = selector.map(selectors::parse).transpose()?;

    // Source line lookup is precomputed once, including CRLF/UTF-8 inputs.
    let mut lines = vec![0usize];
    lines.extend(
        source
            .bytes()
            .enumerate()
            .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
    );
    let line = |offset: usize| lines.partition_point(|start| *start <= offset);
    let mut parents = vec![None; dom.nodes().len()];
    let mut names = vec![String::new(); dom.nodes().len()];
    let mut ids = vec![String::new(); dom.nodes().len()];
    let mut ranges = vec![None; dom.nodes().len()];
    let mut raw_attrs = vec![Vec::new(); dom.nodes().len()];
    let mut count = 0;
    for (i, node) in dom.nodes().iter().enumerate() {
        context.check()?;
        let Some(tag) = node.as_tag() else {
            continue;
        };
        count += 1;
        if count > 4000 || tag.attributes().len() > 256 {
            return Err(LIMIT);
        }
        if tag.raw().as_bytes().is_empty() {
            return Err(("HTML_SPAN", "HTML element has no complete source range"));
        }
        let (start, inclusive_end) = tag.boundaries(parser);
        let end = inclusive_end.checked_add(1).ok_or(LIMIT)?;
        source
            .get(start..end)
            .filter(|raw| raw.starts_with('<') && !raw.is_empty())
            .ok_or(("HTML_SPAN", "HTML parser returned an unsafe source range"))?;
        if projected.as_bytes().get(start..end) != Some(tag.raw().as_bytes()) {
            return Err((
                "HTML_SPAN",
                "HTML parser source range does not match original bytes",
            ));
        }
        ranges[i] = Some(start..end);
        raw_attrs[i] = raw_attributes(opening_tag(&source[start..end]))?;
        let tag_name = tag.name().as_utf8_str();
        let mut name = format!("html:{tag_name}");
        if let Some(id) = raw_attrs[i]
            .iter()
            .find_map(|(key, value)| key.eq_ignore_ascii_case("id").then_some(*value).flatten())
        {
            if !id.is_empty()
                && id.len() <= 200
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            {
                name.push('#');
                name.push_str(id);
            }
        }
        names[i] = name;
        ids[i] = hash(format!("{digest}:{start}:{end}:html-element").as_bytes())[..24].to_owned();
        for child in tag.children().top().iter() {
            let child = child.get_inner() as usize;
            if child < parents.len() {
                parents[child] = Some(i);
            }
        }
    }
    let mut symbols = Vec::new();
    let mut elements = Vec::new();
    // tl may represent tag-looking strings inside raw-text elements as child nodes.
    // Never expose those strings as selectable HTML elements.
    let mut raw_ranges: Vec<(usize, usize)> = dom
        .nodes()
        .iter()
        .enumerate()
        .filter_map(|(i, node)| {
            let tag = node.as_tag()?;
            let name = tag.name().as_utf8_str();
            if ![
                "script",
                "style",
                "textarea",
                "title",
                "xmp",
                "iframe",
                "noembed",
                "noframes",
                "plaintext",
            ]
            .iter()
            .any(|raw| name.eq_ignore_ascii_case(raw))
            {
                return None;
            }
            let range = ranges[i].as_ref()?;
            Some((
                range.start + opening_tag(&source[range.clone()]).len(),
                range.end,
            ))
        })
        .collect();
    raw_ranges.sort_unstable();
    let mut raw_ends = Vec::with_capacity(raw_ranges.len());
    let mut max_end = 0;
    for (_, end) in &raw_ranges {
        max_end = max_end.max(*end);
        raw_ends.push(max_end);
    }
    let hidden = |start: usize| {
        let at = raw_ranges.partition_point(|(from, _)| *from <= start);
        at > 0 && start < raw_ends[at - 1]
    };
    let mut visible_count = 0;
    for (i, node) in dom.nodes().iter().enumerate() {
        context.check()?;
        let Some(tag) = node.as_tag() else {
            continue;
        };
        let range = ranges[i].clone().expect("tag range checked above");
        if hidden(range.start) {
            continue;
        }
        visible_count += 1;
        if let Some(selected) = &selected {
            let mut current = Some(i);
            let mut wanted = selected.len();
            let mut steps = 0;
            while let Some(index) = current {
                steps += 1;
                if steps > dom.nodes().len() {
                    return Err(("HTML_PARSE", "Cyclic HTML parent relationship"));
                }
                if let Some(element) = dom.nodes()[index].as_tag() {
                    if selected[wanted - 1]
                        .matches(&element.name().as_utf8_str(), &raw_attrs[index])
                    {
                        wanted -= 1;
                        if wanted == 0 {
                            break;
                        }
                    } else if index == i {
                        break;
                    }
                }
                current = parents[index];
            }
            if wanted != 0 {
                continue;
            }
        }
        let opening = opening_tag(&source[range.clone()]);
        let attributes = raw_attrs[i]
            .iter()
            .map(|(key, value)| match value {
                Some(value) => format!("{key}=\"{}\"", prefix(value, 160)),
                None => (*key).to_owned(),
            })
            .collect();
        if include_structure {
            elements.push(json!({"symbolId":ids[i],"tag":tag.name().as_utf8_str(),
                "parentSymbolId":parents[i].map(|parent| &ids[parent]),
                "start":line(range.start),"end":line(range.end.saturating_sub(1))}));
        }
        symbols.push(Symbol {
            name: names[i].clone(),
            container: parents[i]
                .map(|parent| names[parent].clone())
                .unwrap_or_default(),
            kind: "html-element",
            start: line(range.start),
            end: line(range.end.saturating_sub(1)),
            signature: prefix(opening, 400).to_owned(),
            attributes,
            doc: String::new(),
            id: ids[i].clone(),
            bytes: range,
        });
    }
    let structure = include_structure.then(|| json!({"parser":"tl","parserVersion":"0.7.8",
        "elements":elements,"elementCount":visible_count,"validation":"none",
        "coverage":"Parser-visible HTML elements and attributes; permissive parsing may omit malformed markup. Angular bindings are preserved as text, not compiled. Attribute values and signatures are previews; full reads preserve original bytes."}));
    Ok((symbols, structure))
}

// Preserve original offsets while shielding raw script/style text from tl's
// permissive tag handling. Only ASCII '<' bytes in bodies are replaced.
fn project_raw_text<'a>(
    source: &'a str,
    context: &OperationContext,
) -> Result<std::borrow::Cow<'a, str>, Failure> {
    use std::borrow::Cow;
    let bytes = source.as_bytes();
    let mut projected: Option<Vec<u8>> = None;
    mask_angular_expressions(source, &mut projected, context)?;
    let mut at = 0;
    while at < source.len() {
        context.check()?;
        let Some(next) = bytes[at..].iter().enumerate().find_map(|(offset, byte)| {
            (*byte == b'<' && projected.as_ref().is_none_or(|p| p[at + offset] == b'<'))
                .then_some(offset)
        }) else {
            break;
        };
        at += next;
        if source[at..].starts_with("<!--") {
            at = source[at + 4..]
                .find("-->")
                .map_or(source.len(), |n| at + 4 + n + 3);
            continue;
        }
        let opening = opening_tag(&source[at..]);
        if !opening.ends_with('>') {
            break;
        }
        let name_start = at + 1;
        let name_len = bytes[name_start..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric())
            .count();
        let name = &source[name_start..name_start + name_len];
        let body = at + opening.len();
        if ![
            "script",
            "style",
            "textarea",
            "title",
            "xmp",
            "iframe",
            "noembed",
            "noframes",
            "plaintext",
        ]
        .iter()
        .any(|raw| name.eq_ignore_ascii_case(raw))
            || opening.ends_with("/>")
        {
            at = body;
            continue;
        }
        let mut close = body;
        let end = loop {
            let Some(next) = source[close..].find("</") else {
                break source.len();
            };
            close += next;
            let from = close + 2;
            let to = from + name.len();
            if source
                .get(from..to)
                .is_some_and(|s| s.eq_ignore_ascii_case(name))
                && bytes
                    .get(to)
                    .is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'>' | b'/'))
            {
                break close;
            }
            close += 2;
        };
        for i in body..end {
            if bytes[i] == b'<' {
                projected.get_or_insert_with(|| bytes.to_vec())[i] = b' ';
            }
        }
        at = end;
        if at == source.len() {
            break;
        }
        at += opening_tag(&source[at..]).len();
    }
    Ok(match projected {
        Some(bytes) => {
            Cow::Owned(String::from_utf8(bytes).expect("ASCII-only projection preserves UTF-8"))
        }
        None => Cow::Borrowed(source),
    })
}

// Attribute previews come from the original opening tag. tl normalizes Angular
// punctuation, so its attribute map must not be used to report binding names.
fn mask_angular_expressions(
    source: &str,
    projected: &mut Option<Vec<u8>>,
    context: &OperationContext,
) -> Result<(), Failure> {
    let bytes = source.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if at % 4096 == 0 {
            context.check()?;
        }
        let interpolation = bytes.get(at..at + 2) == Some(b"{{");
        let header = if bytes[at] == b'@' {
            ["if", "for", "switch", "case", "defer"]
                .iter()
                .find_map(|word| {
                    let end = at + 1 + word.len();
                    if source.get(at + 1..end) != Some(*word) {
                        return None;
                    }
                    let mut open = end;
                    while bytes.get(open).is_some_and(u8::is_ascii_whitespace) {
                        open += 1;
                    }
                    (bytes.get(open) == Some(&b'(')).then_some(open)
                })
        } else {
            None
        };
        if !interpolation && header.is_none() {
            at += 1;
            continue;
        }
        let start = header.map_or(at + 2, |open| open + 1);
        let mut end = start;
        let mut depth = 0usize;
        let mut quote = 0;
        while end < bytes.len() {
            let byte = bytes[end];
            if quote != 0 {
                if byte == b'\\' {
                    end = (end + 2).min(bytes.len());
                    continue;
                }
                if byte == quote {
                    quote = 0;
                }
            } else if matches!(byte, b'\'' | b'"' | b'`') {
                quote = byte;
            } else if (interpolation && byte == b'{') || (!interpolation && byte == b'(') {
                depth += 1;
            } else if (interpolation && byte == b'}') || (!interpolation && byte == b')') {
                if depth == 0 && (!interpolation || bytes.get(end + 1) == Some(&b'}')) {
                    break;
                }
                depth = depth.saturating_sub(1);
            }
            end += 1;
        }
        for i in start..end {
            if bytes[i] == b'<' {
                projected.get_or_insert_with(|| bytes.to_vec())[i] = b' ';
            }
        }
        at = (end + if interpolation { 2 } else { 1 }).min(bytes.len());
    }
    Ok(())
}

fn raw_attributes(opening: &str) -> Result<Vec<(&str, Option<&str>)>, Failure> {
    let bytes = opening.as_bytes();
    let mut at = 1;
    while at < bytes.len() && !bytes[at].is_ascii_whitespace() && !matches!(bytes[at], b'>' | b'/')
    {
        at += 1;
    }
    let mut attributes = Vec::new();
    while at < bytes.len() {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if at == bytes.len() || matches!(bytes[at], b'>' | b'/') {
            break;
        }
        let start = at;
        while at < bytes.len()
            && !bytes[at].is_ascii_whitespace()
            && !matches!(bytes[at], b'=' | b'>')
        {
            at += 1;
        }
        let name = &opening[start..at];
        if name.is_empty() {
            at += 1;
            continue;
        }
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if bytes.get(at) != Some(&b'=') {
            attributes.push((name, None));
        } else {
            at += 1;
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            let quote = bytes.get(at).copied().filter(|b| matches!(b, b'\'' | b'"'));
            if quote.is_some() {
                at += 1;
            }
            let start = at;
            while at < bytes.len()
                && match quote {
                    Some(quote) => bytes[at] != quote,
                    None => !bytes[at].is_ascii_whitespace() && bytes[at] != b'>',
                }
            {
                at += 1;
            }
            attributes.push((name, Some(&opening[start..at])));
            if quote.is_some() && at < bytes.len() {
                at += 1;
            }
        }
        if attributes.len() > 256 {
            return Err(LIMIT);
        }
    }
    Ok(attributes)
}

fn opening_tag(raw: &str) -> &str {
    let mut quote = 0;
    for (i, byte) in raw.bytes().enumerate() {
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = byte;
        } else if byte == b'>' {
            return &raw[..i + 1];
        }
    }
    raw
}
