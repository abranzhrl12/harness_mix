use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};

const CONTRACT: Failure = ("EXTRACTION_CONTRACT", "Invalid HTTP route buffers");

/// Adds literal ASP.NET Core route metadata to C# method nodes.
pub(super) fn enrich(
    _path: &str,
    source: &str,
    graph: &mut Value,
    context: &OperationContext,
) -> Result<(), Failure> {
    let mask = mask_non_code(source);
    let nodes = graph["nodes"].as_array_mut().ok_or(CONTRACT)?;
    let classes: Vec<(String, String, usize, Vec<String>, bool)> = nodes
        .iter()
        .filter(|node| node["kind"] == "class")
        .filter_map(|node| {
            let name = node["name"].as_str()?.to_owned();
            let qualified = node["qualifiedName"].as_str()?.to_owned();
            let start_line = node["startLine"].as_u64()? as usize;
            let end_line = node["endLine"].as_u64()? as usize;
            let declaration = find_class_declaration(&mask, &name, start_line, end_line)?;
            let attrs = declaration_attributes(source, &mask, declaration);
            let brace = mask
                .get(declaration..)?
                .iter()
                .position(|b| *b == b'{')
                .map(|n| declaration + n)?;
            if !is_controller_base(source.get(declaration..brace)?) {
                return None;
            }
            let route_declared = has_attribute(&attrs, "Route");
            let parsed_routes = parse_attribute_routes(&attrs, "Route");
            if parsed_routes.iter().any(Option::is_none) {
                return None;
            }
            let routes = parsed_routes.into_iter().flatten().collect();
            Some((name, qualified, end_line, routes, route_declared))
        })
        .collect();

    for (controller_name, qualified, class_end, class_routes, class_route_declared) in classes {
        context.check()?;
        if class_route_declared && class_routes.is_empty() {
            continue;
        }
        let controller = controller_name
            .strip_suffix("Controller")
            .unwrap_or(&controller_name);
        let action_indexes: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node["kind"] == "method" && node["visibility"] == "public")
            .filter(|(_, node)| node["isStatic"] != true && node["isAbstract"] != true)
            .filter(|(_, node)| {
                let q = node["qualifiedName"].as_str().unwrap_or("");
                let name = node["name"].as_str().unwrap_or("");
                let rest = q
                    .strip_prefix(&format!("{qualified}."))
                    .or_else(|| q.strip_prefix(&format!("{qualified}::")));
                rest.is_some_and(|rest| {
                    rest.trim_start_matches(['.', ':']).split(['.', ':']).next() == Some(name)
                }) && name != controller_name
                    && node["endLine"].as_u64().unwrap_or(0) as usize <= class_end
            })
            .map(|(index, _)| index)
            .collect();

        for index in action_indexes {
            context.check()?;
            let method_name = nodes[index]["name"].as_str().ok_or(CONTRACT)?.to_owned();
            let line = nodes[index]["startLine"].as_u64().ok_or(CONTRACT)? as usize;
            let end_line = nodes[index]["endLine"].as_u64().unwrap_or(line as u64) as usize;
            let declaration = find_method_declaration(&mask, &method_name, line, end_line);
            let offset = line_start(source, line).ok_or(CONTRACT)?;
            let attrs = declaration
                .map(|at| declaration_attributes(source, &mask, at))
                .unwrap_or_else(|| preceding_attributes(source, &mask, offset));
            if has_attribute(&attrs, "NonAction") {
                continue;
            }

            let verbs = parse_http_routes(&attrs);
            let action_routes = parse_attribute_routes(&attrs, "Route");
            if action_routes.iter().any(Option::is_none) || has_dynamic_http_template(&attrs) {
                continue;
            }
            let specs: Vec<(Option<String>, Option<String>)> = if !verbs.is_empty() {
                verbs
                    .into_iter()
                    .flat_map(|(verb, path)| {
                        if path.is_none() && !action_routes.is_empty() {
                            action_routes
                                .iter()
                                .cloned()
                                .map(|suffix| (verb.clone(), suffix))
                                .collect::<Vec<_>>()
                        } else {
                            vec![(verb, path)]
                        }
                    })
                    .collect()
            } else if !action_routes.is_empty() {
                action_routes.into_iter().map(|p| (None, p)).collect()
            } else {
                vec![(None, None)]
            };

            let mut routes = Vec::new();
            let prefixes: Vec<Option<&str>> = if class_routes.is_empty() {
                vec![None]
            } else {
                class_routes
                    .iter()
                    .map(|prefix| Some(prefix.as_str()))
                    .collect()
            };
            for prefix in prefixes {
                for (verb, path) in &specs {
                    let resolved = if let Some(prefix) = prefix {
                        combine(Some(prefix), path.as_deref())
                    } else if let Some(path) = path.as_deref() {
                        Some(path.to_owned())
                    } else {
                        Some(format!("{controller}/{method_name}"))
                    };
                    if let Some(path) = resolved.filter(|p| literal_path(p)) {
                        routes.push(json!({"path": normalize_path(&path), "method": verb}));
                    }
                }
            }
            let mut unique = Vec::new();
            for route in routes {
                if !unique.contains(&route) {
                    unique.push(route);
                }
            }
            if unique.is_empty() {
                continue;
            }

            nodes[index]["httpRoutes"] = json!(unique);
            if !nodes[index]["decorators"].is_array() {
                nodes[index]["decorators"] = json!([]);
            }
            let decorators = nodes[index]["decorators"].as_array_mut().ok_or(CONTRACT)?;
            for route in unique {
                let sentinel = format!(
                    "harness:http:{}",
                    json!({"path":route["path"], "method":route["method"]})
                );
                if !decorators
                    .iter()
                    .any(|item| item.as_str() == Some(&sentinel))
                {
                    decorators.push(json!(sentinel));
                }
            }
        }
    }
    Ok(())
}

fn line_start(source: &str, line: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    if line == 1 {
        return Some(0);
    }
    source
        .match_indices('\n')
        .nth(line - 2)
        .map(|(at, _)| at + 1)
}

fn line_end(source: &str, line: usize) -> Option<usize> {
    let start = line_start(source, line)?;
    Some(
        source[start..]
            .find('\n')
            .map(|offset| start + offset)
            .unwrap_or(source.len()),
    )
}

fn find_class_declaration(
    mask: &[u8],
    name: &str,
    start_line: usize,
    end_line: usize,
) -> Option<usize> {
    let start = line_start(std::str::from_utf8(mask).ok()?, start_line)?;
    let end = line_end(std::str::from_utf8(mask).ok()?, end_line)?;
    let region = mask.get(start..end)?;
    for (offset, _) in region.iter().enumerate().filter(|(_, byte)| **byte == b'c') {
        let at = start + offset;
        if mask.get(at..at + 5)? != b"class" {
            continue;
        }
        if at > 0 && is_identifier(mask[at - 1]) {
            continue;
        }
        let mut cursor = at + 5;
        if mask.get(cursor).is_some_and(|byte| is_identifier(*byte)) {
            continue;
        }
        while mask
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            cursor += 1;
        }
        if mask.get(cursor..cursor + name.len()) == Some(name.as_bytes())
            && mask
                .get(cursor + name.len())
                .is_none_or(|byte| !is_identifier(*byte))
        {
            return Some(at);
        }
    }
    None
}

fn find_method_declaration(
    mask: &[u8],
    name: &str,
    start_line: usize,
    end_line: usize,
) -> Option<usize> {
    let text = std::str::from_utf8(mask).ok()?;
    let start = line_start(text, start_line)?;
    let end = line_end(text, end_line)?;
    let region = mask.get(start..end)?;
    for (offset, _) in region
        .windows(name.len())
        .enumerate()
        .filter(|(_, bytes)| *bytes == name.as_bytes())
    {
        let at = start + offset;
        if at > 0 && is_identifier(mask[at - 1]) {
            continue;
        }
        if mask
            .get(at + name.len())
            .is_some_and(|byte| is_identifier(*byte))
        {
            continue;
        }
        let mut cursor = at + name.len();
        while mask
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            cursor += 1;
        }
        if mask.get(cursor) == Some(&b'(') {
            return Some(at);
        }
    }
    None
}

fn is_identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn declaration_attributes(source: &str, mask: &[u8], declaration: usize) -> String {
    let line = source[..declaration]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1;
    let line_offset = line_start(source, line).unwrap_or(0);
    let mut attrs = preceding_attributes(source, mask, line_offset);
    let boundary = mask[line_offset..declaration]
        .iter()
        .rposition(|b| matches!(*b, b'}' | b';' | b'{'))
        .map(|offset| line_offset + offset + 1)
        .unwrap_or(line_offset);
    if let Some(relative) = mask[boundary..declaration]
        .iter()
        .position(|byte| *byte == b'[')
    {
        let start = boundary + relative;
        let inline = source.get(start..declaration).unwrap_or("");
        if !attrs.is_empty() {
            attrs.push('\n');
        }
        attrs.push_str(inline);
    }
    attrs
}

fn has_attribute(attrs: &str, expected: &str) -> bool {
    attribute_names(attrs).iter().any(|name| name == expected)
}

fn has_dynamic_http_template(attrs: &str) -> bool {
    [
        "HttpGet",
        "HttpPost",
        "HttpPut",
        "HttpPatch",
        "HttpDelete",
        "HttpHead",
        "HttpOptions",
    ]
    .iter()
    .any(|name| {
        parse_attribute_routes(attrs, name)
            .iter()
            .any(Option::is_none)
    })
}

/// Preserve source offsets while blanking comments and string/character bodies.
fn mask_non_code(source: &str) -> Vec<u8> {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        if bytes[at] == b'/' && bytes.get(at + 1) == Some(&b'/') {
            at += 2;
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
        } else if bytes[at] == b'/' && bytes.get(at + 1) == Some(&b'*') {
            at += 2;
            while at + 1 < bytes.len() && !(bytes[at] == b'*' && bytes[at + 1] == b'/') {
                at += 1;
            }
            at = (at + 2).min(bytes.len());
        } else if bytes[at] == b'"' || bytes[at] == b'\'' {
            let quote = bytes[at];
            at += 1;
            while at < bytes.len() {
                if bytes[at] == b'\\' {
                    at = (at + 2).min(bytes.len());
                } else if bytes[at] == quote {
                    at += 1;
                    break;
                } else {
                    at += 1;
                }
            }
        } else {
            at += 1;
            continue;
        }
        for byte in &mut masked[start..at] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    masked
}

/// Collect adjacent bracketed attributes immediately above a declaration.
fn preceding_attributes(source: &str, mask: &[u8], declaration: usize) -> String {
    let mut at = declaration.min(mask.len());
    while at > 0 && matches!(mask[at - 1], b' ' | b'\t' | b'\r' | b'\n') {
        at -= 1;
    }
    let mut start = at;
    loop {
        if at == 0 || mask[at - 1] != b']' {
            break;
        }
        let mut depth = 0usize;
        let mut cursor = at;
        let mut opening = None;
        while cursor > 0 {
            cursor -= 1;
            match mask[cursor] {
                b']' => depth += 1,
                b'[' => {
                    depth -= 1;
                    if depth == 0 {
                        opening = Some(cursor);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(open) = opening else {
            break;
        };
        start = open;
        at = open;
        while at > 0 && matches!(mask[at - 1], b' ' | b'\t' | b'\r' | b'\n') {
            at -= 1;
        }
    }
    source.get(start..declaration).unwrap_or("").to_owned()
}

fn attribute_items(attrs: &str) -> Vec<&str> {
    let mut items = Vec::new();
    for list in attrs.split('[').skip(1) {
        let Some(list) = list.split(']').next() else {
            continue;
        };
        let mut start = 0;
        let mut parens = 0usize;
        let mut quote = None;
        let mut escaped = false;
        for (at, ch) in list.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' && quote.is_some() {
                escaped = true;
                continue;
            }
            if let Some(active) = quote {
                if ch == active {
                    quote = None;
                }
                continue;
            }
            if ch == '"' || ch == '\'' {
                quote = Some(ch);
            } else if ch == '(' {
                parens += 1;
            } else if ch == ')' {
                parens = parens.saturating_sub(1);
            } else if ch == ',' && parens == 0 {
                items.push(list[start..at].trim());
                start = at + ch.len_utf8();
            }
        }
        let tail = list[start..].trim();
        if !tail.is_empty() {
            items.push(tail);
        }
    }
    items
}

fn attribute_names(attrs: &str) -> Vec<String> {
    attribute_items(attrs)
        .into_iter()
        .filter_map(|item| {
            let name = item
                .trim_start()
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
                .next()?;
            let short = name.rsplit('.').next().unwrap_or(name);
            Some(short.strip_suffix("Attribute").unwrap_or(short).to_owned())
        })
        .collect()
}

fn parse_attribute_routes(attrs: &str, expected: &str) -> Vec<Option<String>> {
    let mut output = Vec::new();
    for item in attribute_items(attrs) {
        let Some((head, args)) = item.split_once('(') else {
            continue;
        };
        let head = head.trim();
        let short = head.rsplit('.').next().unwrap_or(head);
        let short = short.strip_suffix("Attribute").unwrap_or(short);
        if short != expected {
            continue;
        }
        let Some(close) = closing_paren(args) else {
            output.push(None);
            continue;
        };
        let argument = &args[..close];
        output.push(if argument.trim().is_empty() {
            Some(String::new())
        } else {
            literal_argument(argument)
        });
    }
    output
}

fn closing_paren(args: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    for (at, ch) in args.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quote.is_some() {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch == ')' {
            return Some(at);
        }
    }
    None
}

fn parse_http_routes(attrs: &str) -> Vec<(Option<String>, Option<String>)> {
    let mut routes = Vec::new();
    for (attribute, verb) in [
        ("HttpGet", "GET"),
        ("HttpPost", "POST"),
        ("HttpPut", "PUT"),
        ("HttpPatch", "PATCH"),
        ("HttpDelete", "DELETE"),
        ("HttpHead", "HEAD"),
        ("HttpOptions", "OPTIONS"),
    ] {
        let found = attribute_items(attrs)
            .into_iter()
            .filter(|item| {
                let name = item
                    .split_once('(')
                    .map(|pair| pair.0.trim())
                    .unwrap_or(item.trim())
                    .rsplit('.')
                    .next()
                    .unwrap_or("");
                name.strip_suffix("Attribute").unwrap_or(name) == attribute
            })
            .collect::<Vec<_>>();
        for item in found {
            let path = if let Some((_, args)) = item.split_once('(') {
                closing_paren(args).and_then(|close| literal_argument(&args[..close]))
            } else {
                None
            };
            routes.push((Some(verb.to_owned()), path));
        }
    }
    routes
}

fn literal_argument(args: &str) -> Option<String> {
    let arg = args.trim().strip_prefix('@').unwrap_or(args.trim());
    let quoted = arg.strip_prefix('"')?;
    let (value, remainder) = quoted.split_once('"')?;
    let remainder = remainder.trim();
    if !remainder.is_empty() && !remainder.starts_with(',') {
        return None;
    }
    if value.contains(['\\', '{', '}', '[', ']', '$']) {
        return None;
    }
    Some(value.to_owned())
}

fn is_controller_base(header: &str) -> bool {
    let Some((_, bases)) = header.split_once(':') else {
        return false;
    };
    bases.split(',').any(|base| {
        let name = base
            .trim()
            .split('<')
            .next()
            .unwrap_or("")
            .trim()
            .rsplit('.')
            .next()
            .unwrap_or("");
        matches!(name, "Controller" | "ControllerBase" | "ApiController")
    })
}

fn combine(prefix: Option<&str>, suffix: Option<&str>) -> Option<String> {
    match (prefix, suffix) {
        (Some(prefix), Some(suffix)) if prefix.is_empty() => Some(suffix.to_owned()),
        (Some(_), Some(suffix)) if suffix.starts_with("~/") => {
            Some(format!("/{}", suffix.trim_start_matches("~/")))
        }
        (Some(_), Some(suffix)) if suffix.starts_with('/') => Some(suffix.to_owned()),
        (Some(prefix), Some(suffix)) => Some(format!(
            "{}/{}",
            prefix.trim_end_matches('/'),
            suffix.trim_start_matches('/')
        )),
        (Some(prefix), None) => Some(prefix.to_owned()),
        (None, Some(suffix)) => Some(suffix.to_owned()),
        (None, None) => None,
    }
}

fn literal_path(path: &str) -> bool {
    if path.contains(['{', '}', '[', ']', '$']) || path.contains("://") {
        return false;
    }
    let scheme = path.split_once(':').map(|(prefix, _)| prefix);
    !scheme.is_some_and(|scheme| {
        let scheme = scheme.to_ascii_lowercase();
        matches!(
            scheme.as_str(),
            "http" | "https" | "javascript" | "mailto" | "data" | "file" | "ftp" | "ws" | "wss"
        )
    })
}

fn normalize_path(path: &str) -> String {
    let path = path.strip_prefix("~/").unwrap_or(path);
    format!("/{}", path.trim_matches('/'))
}

#[cfg(test)]
#[path = "http_backend_tests.rs"]
mod tests;
