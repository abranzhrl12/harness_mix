//! Redacted configuration skeletons. Raw values and parser errors never enter results.
use super::project::Project;
use harness_domain::{
    Failure,
    contracts::ConfigArgs,
    execution::OperationContext,
    results::{prefix, utf16_len},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn safe_name(name: &str) -> &str {
    let lower = name.to_ascii_lowercase();
    if name.len() > 80
        || ["sk-", "ghp_", "github_pat_", "akia"]
            .iter()
            .any(|p| lower.starts_with(p))
    {
        "redactedName"
    } else {
        name
    }
}
pub(super) fn section(
    project: &Project,
    args: &ConfigArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let max = args.max_chars.unwrap_or(8000);
    if !(500..=12000).contains(&max)
        || args.section.is_empty()
        || args.section.len() > 100
        || !args
            .section
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err((
            "ARGUMENT",
            "Config requires section name <=100 ASCII identifier characters, maxChars 500..12000",
        ));
    }
    let extension = std::path::Path::new(&args.path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["json", "config", "xml", "csproj", "props", "targets"].contains(&extension.as_str()) {
        return Err((
            "ARGUMENT",
            "Config supports .json/.config/.xml/.csproj/.props/.targets only",
        ));
    }
    let bytes = project.redacted_input_bytes(&args.path, context, 1024 * 1024)?;
    let source = std::str::from_utf8(&bytes).map_err(|_| ("ENCODING", "Config must be UTF-8"))?;
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    if extension == "json" {
        return json_section(source, args, max, context);
    }
    if source.contains("<!DOCTYPE") || source.contains("<!ENTITY") {
        return Err(("CONFIG_XML", "DTD/entity declarations are not accepted"));
    }
    context.check()?;

    if args.raw.unwrap_or(false) {
        return raw_iis_section(source, args, max, context);
    }

    let document = roxmltree::Document::parse_with_options(
        source,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 10000,
            entity_resolver: None,
        },
    )
    .map_err(|_| {
        (
            "CONFIG_XML",
            "Invalid XML or parser budget exceeded; original error withheld",
        )
    })?;
    context.check()?;
    for node in document.descendants() {
        if node.ancestors().take(66).count() > 65 {
            return Err(("CONFIG_XML", "XML depth exceeds 64"));
        }
    }
    let mut matching = document.descendants().filter(|node| {
        node.is_element() && node.tag_name().name().eq_ignore_ascii_case(&args.section)
    });
    let selected = matching
        .next()
        .ok_or(("SECTION", "XML section not found"))?;
    if matching.next().is_some() {
        return Err((
            "AMBIGUOUS_SECTION",
            "Multiple sections match; select an unambiguous section",
        ));
    }
    let mut output = String::new();
    let mut bounded = false;
    let mut pending = vec![(selected, false)];
    while let Some((node, closing)) = pending.pop() {
        context.check()?;
        match (node.is_element(), closing) {
            (true, false) => {
                output.push('<');
                output.push_str(safe_name(node.tag_name().name()));
                let mut names = BTreeSet::new();
                for attribute in node.attributes() {
                    let name = safe_name(attribute.name());
                    if names.insert(name) {
                        output.push(' ');
                        output.push_str(name);
                        output.push_str("=\"***\"");
                    }
                    if output.len() > 64000 {
                        bounded = true;
                        break;
                    }
                }
                output.push('>');
                pending.push((node, true));
                for child in node.children().collect::<Vec<_>>().into_iter().rev() {
                    pending.push((child, false));
                }
            }
            (false, false) if node.is_text() => {
                if node.text().is_some_and(|s| !s.trim().is_empty()) {
                    output.push_str("***");
                }
            }
            (true, true) => {
                output.push_str("</");
                output.push_str(safe_name(node.tag_name().name()));
                output.push('>');
            }
            _ => {}
        }
        if bounded || output.len() > 64000 {
            bounded = true;
            break;
        }
    }
    context.check()?;
    let truncated = bounded || utf16_len(&output) > max;
    let mut result = json!({"path":args.path,"section":args.section,"redacted":true,"redactionPolicy":"all_attribute_and_text_values",
        "text":prefix(&output,max),"truncated":truncated,"externalReferencesFollowed":false,"editReady":false,
        "namespacePolicy":"local names only; repeated local attributes collapsed","complete":!bounded,"_complete":!bounded});
    if truncated {
        result["_detail"] = json!({"text":output,"redacted":true,"complete":!bounded});
    }
    Ok(result)
}

fn raw_iis_section(
    source: &str,
    args: &ConfigArgs,
    max: usize,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let section_name = &args.section;
    let lower_source = source.to_ascii_lowercase();
    let lower_name = section_name.to_ascii_lowercase();

    let open_tag = format!("<{}", lower_name);
    let mut search_idx = 0;
    let mut matched_span: Option<(usize, usize)> = None;

    while let Some(start_rel) = lower_source[search_idx..].find(&open_tag) {
        let tag_start = search_idx + start_rel;
        let char_after = source[tag_start + open_tag.len()..].chars().next();

        if let Some(ch) = char_after
            && (ch.is_whitespace() || ch == '>' || ch == '/')
            && let Some(tag_end_rel) = source[tag_start..].find('>')
        {
            let tag_end = tag_start + tag_end_rel;
            if source[tag_start..=tag_end].ends_with("/>") {
                if matched_span.is_some() {
                    return Err((
                        "AMBIGUOUS_SECTION",
                        "Multiple sections match; select an unambiguous section",
                    ));
                }
                matched_span = Some((tag_start, tag_end + 1));
            } else {
                let close_tag = format!("</{}>", lower_name);
                let close_tag_ws = format!("</{} ", lower_name);
                let close_rel = lower_source[tag_end..]
                    .find(&close_tag)
                    .or_else(|| lower_source[tag_end..].find(&close_tag_ws));

                if let Some(c_rel) = close_rel {
                    let close_start = tag_end + c_rel;
                    if let Some(end_rel) = source[close_start..].find('>') {
                        let total_end = close_start + end_rel + 1;
                        if matched_span.is_some() {
                            return Err((
                                "AMBIGUOUS_SECTION",
                                "Multiple sections match; select an unambiguous section",
                            ));
                        }
                        matched_span = Some((tag_start, total_end));
                    }
                }
            }
        }
        search_idx = tag_start + open_tag.len();
    }

    let (start, end) = matched_span.ok_or((
        "SECTION",
        "XML section not found; usar harness_search kind:text para listar secciones",
    ))?;
    context.check()?;

    let raw_text = &source[start..end];
    let redacted = redact_iis_heuristics(raw_text);

    let bounded = redacted.len() > 64000;
    let output = if bounded {
        prefix(&redacted, 64000)
    } else {
        &redacted
    };

    let truncated = bounded || utf16_len(output) > max;
    let mut result = json!({
        "path": args.path,
        "section": args.section,
        "format": "raw_xml",
        "redacted": true,
        "redactionPolicy": "iis_heuristic_sensitive_attributes_and_tags",
        "text": prefix(output, max),
        "truncated": truncated,
        "externalReferencesFollowed": false,
        "editReady": false,
        "complete": !bounded,
        "_complete": !bounded
    });

    if truncated {
        result["_detail"] = json!({"text": output, "redacted": true, "complete": !bounded});
    }

    Ok(result)
}

fn redact_iis_heuristics(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut i = 0;
    let bytes = text.as_bytes();
    let len = bytes.len();

    let sensitive_keys = [
        "password",
        "secret",
        "connectionstring",
        "pwd",
        "token",
        "clientsecret",
        "apikey",
    ];

    while i < len {
        // Redact <value>...</value>
        if text[i..].starts_with("<value") {
            let rest = &text[i..];
            if let Some(open_end) = rest.find('>') {
                let close_tag = "</value>";
                if let Some(close_start) = rest[open_end..].to_ascii_lowercase().find(close_tag) {
                    let total_close_end = open_end + close_start + close_tag.len();
                    result.push_str("<value>***</value>");
                    i += total_close_end;
                    continue;
                }
            }
        }

        // IIS appSettings pattern: <add key="SecretKey" value="secret_value" ... />
        if text[i..].starts_with("<add")
            && (i + 4 >= len || bytes[i + 4].is_ascii_whitespace())
            && let Some(tag_end_rel) = text[i..].find('>')
        {
            let tag_str = &text[i..i + tag_end_rel + 1];
            let is_sensitive = sensitive_keys.iter().any(|k| {
                if let Some(key_idx) = tag_str.to_ascii_lowercase().find("key") {
                    let after_key = &tag_str[key_idx + 3..];
                    if let Some(eq_idx) = after_key.find('=') {
                        let after_eq = after_key[eq_idx + 1..].trim_start();
                        if after_eq.starts_with('"') || after_eq.starts_with('\'') {
                            let quote = after_eq.chars().next().unwrap();
                            let inner = &after_eq[1..];
                            if let Some(q_end) = inner.find(quote) {
                                let key_name = &inner[..q_end].to_ascii_lowercase();
                                return key_name.contains(k);
                            }
                        }
                    }
                }
                false
            });

            if is_sensitive {
                // Redact value="..." in this tag
                if let Some(val_idx) = tag_str.to_ascii_lowercase().find("value") {
                    let after_val = &tag_str[val_idx + 5..];
                    if let Some(eq_idx) = after_val.find('=') {
                        let after_eq = &after_val[eq_idx + 1..];
                        let ws_count = after_eq.len() - after_eq.trim_start().len();
                        let trimmed = after_eq.trim_start();
                        if trimmed.starts_with('"') || trimmed.starts_with('\'') {
                            let quote = trimmed.chars().next().unwrap();
                            let inner = &trimmed[1..];
                            if let Some(q_end) = inner.find(quote) {
                                let val_start_abs = i + val_idx + 5 + eq_idx + 1 + ws_count + 1;
                                let val_end_abs = val_start_abs + q_end;
                                result.push_str(&text[i..val_start_abs]);
                                result.push_str("***");
                                result.push_str(&text[val_end_abs..i + tag_end_rel + 1]);
                                i += tag_end_rel + 1;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // Direct sensitive attribute redaction: (password="...", connectionString="...", etc.)
        let mut matched_attr = false;
        for key in &sensitive_keys {
            let key_len = key.len();
            if i + key_len < len && text[i..i + key_len].eq_ignore_ascii_case(key) {
                let char_before = if i > 0 {
                    text[..i].chars().next_back()
                } else {
                    None
                };
                let is_word_boundary = char_before
                    .is_none_or(|c| c.is_whitespace() || c == '<' || c == '(' || c == ';');

                if is_word_boundary {
                    let mut j = i + key_len;
                    while j < len
                        && (bytes[j] == b' '
                            || bytes[j] == b'\t'
                            || bytes[j] == b'\r'
                            || bytes[j] == b'\n')
                    {
                        j += 1;
                    }
                    if j < len && bytes[j] == b'=' {
                        j += 1;
                        while j < len
                            && (bytes[j] == b' '
                                || bytes[j] == b'\t'
                                || bytes[j] == b'\r'
                                || bytes[j] == b'\n')
                        {
                            j += 1;
                        }
                        if j < len && (bytes[j] == b'"' || bytes[j] == b'\'') {
                            let quote = bytes[j];
                            let val_start = j + 1;
                            let mut val_end = val_start;
                            while val_end < len && bytes[val_end] != quote {
                                val_end += 1;
                            }
                            if val_end < len {
                                result.push_str(&text[i..val_start]);
                                result.push_str("***");
                                result.push(quote as char);
                                i = val_end + 1;
                                matched_attr = true;
                                break;
                            }
                        }
                    }
                }
            }
        }

        if !matched_attr {
            result.push(text[i..].chars().next().unwrap());
            i += text[i..].chars().next().unwrap().len_utf8();
        }
    }

    result
}

fn json_section(
    source: &str,
    args: &ConfigArgs,
    max: usize,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    // Keep serde's recursion limit enabled; never return its source-bearing errors.
    let document: Value = serde_json::from_str(source).map_err(|_| {
        (
            "CONFIG_JSON",
            "Invalid JSON or parser budget exceeded; original error withheld",
        )
    })?;
    let mut pending = vec![(&document, 0usize)];
    let mut nodes = 0usize;
    while let Some((node, depth)) = pending.pop() {
        context.check()?;
        nodes += 1;
        if nodes > 10000 || depth > 64 {
            return Err(("CONFIG_JSON", "JSON exceeds 10000 nodes or depth 64"));
        }
        match node {
            Value::Array(items) => pending.extend(items.iter().map(|v| (v, depth + 1))),
            Value::Object(items) => pending.extend(items.values().map(|v| (v, depth + 1))),
            _ => {}
        }
    }
    let selected = document
        .as_object()
        .and_then(|object| object.get(&args.section))
        .ok_or((
            "SECTION",
            "JSON top-level section not found (case-sensitive)",
        ))?;
    // JSON property names can themselves be secrets. Do not publish them, even in detail.
    fn redact(node: &Value) -> Value {
        match node {
            Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
            Value::Object(items) => Value::Object(
                items
                    .values()
                    .enumerate()
                    .map(|(index, value)| (format!("field_{}", index + 1), redact(value)))
                    .collect(),
            ),
            _ => Value::String("***".into()),
        }
    }
    let sanitized = redact(selected);
    context.check()?;
    let output = serde_json::to_string(&sanitized)
        .map_err(|_| ("CONFIG_JSON", "Redacted JSON serialization failed"))?;
    let bounded = output.len() > 64000;
    // Truncation is only ever applied after redaction. A partial skeleton is not valid JSON.
    let output = if bounded {
        prefix(&output, 16000)
    } else {
        &output
    };
    let truncated = bounded || utf16_len(output) > max;
    let mut result = json!({"path":args.path,"section":args.section,"format":"json",
        "redacted":true,"redactionPolicy":"all_property_names_and_scalar_values",
        "text":prefix(output,max),"truncated":truncated,"externalReferencesFollowed":false,
        "editReady":false,"complete":!bounded,"_complete":!bounded,
        "sectionPolicy":"case-sensitive top-level property; duplicate keys use last value"});
    if truncated {
        result["_detail"] = json!({"text":output,"redacted":true,"complete":!bounded});
    }
    Ok(result)
}
