//! XML skeleton only. Raw XML values and parser error messages never enter results/details/logs.
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
    if !["config", "xml", "csproj", "props", "targets"].contains(&extension.as_str()) {
        return Err((
            "ARGUMENT",
            "Config supports .config/.xml/.csproj/.props/.targets only",
        ));
    }
    let bytes = project.redacted_input_bytes(&args.path, context, 1024 * 1024)?;
    let source = std::str::from_utf8(&bytes).map_err(|_| ("ENCODING", "XML must be UTF-8"))?;
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
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
