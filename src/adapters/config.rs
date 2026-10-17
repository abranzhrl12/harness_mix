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
