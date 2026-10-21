//! Project analyzed DI sites onto unique native class anchors.
use super::{Facts, INVALID, LIMIT, analyze};
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn enrich(
    path: &str,
    source: &str,
    graph: &mut Value,
    context: &OperationContext,
) -> Result<(), Failure> {
    if !source.contains('@') && !source.contains("inject(") {
        return Ok(());
    }
    let facts = analyze(path, source, context)?;
    if facts.sites.is_empty() {
        return Ok(());
    }
    project(path, graph, facts, context)
}

fn project(
    path: &str,
    graph: &mut Value,
    facts: Facts,
    context: &OperationContext,
) -> Result<(), Failure> {
    let nodes = graph["nodes"].as_array().ok_or(INVALID)?;
    let mut refs = Vec::new();
    let mut anchors = BTreeMap::<(String, i64, i64), Vec<String>>::new();
    for node in nodes.iter().filter(|node| node["kind"] == "class") {
        if let (Some(name), Some(line), Some(col), Some(id)) = (
            node["name"].as_str(),
            node["startLine"].as_i64(),
            node["startColumn"].as_i64(),
            node["id"].as_str(),
        ) {
            anchors
                .entry((name.to_owned(), line, col))
                .or_default()
                .push(id.to_owned());
        }
    }
    for ((name, line, col), binding) in facts.sites {
        context.check()?;
        let Some(ids) = anchors.get(&(
            binding.class_name.clone(),
            binding.class_line,
            binding.class_col,
        )) else {
            continue;
        };
        if ids.len() != 1 {
            continue;
        }
        refs.push(json!({
            "fromNodeId": ids[0],
            "referenceName": name,
            "referenceKind": "angular_injects",
            "line": line,
            "column": col,
            "filePath": path,
            "metadata": {"framework":"angular","module":binding.module,"imported":binding.imported,"tokenForm":binding.token_form}
        }));
    }
    let emitted = refs.len();
    if graph["unresolvedReferences"]
        .as_array()
        .ok_or(INVALID)?
        .len()
        .saturating_add(emitted)
        > 20_000
    {
        return Err(LIMIT);
    }
    graph["unresolvedReferences"]
        .as_array_mut()
        .ok_or(INVALID)?
        .append(&mut refs);
    if !graph["coverage"].is_object() {
        graph["coverage"] = json!({"parser":"angular-token-dependency-subset-v1"});
    }
    graph["coverage"]["di"] = json!({
        "tokenReferencesEmitted":emitted,
        "proof":"lexical-value-import-token-dependency",
        "runtimeProviderSelectionEvaluated":false,
        "importedClassResolution":"requires_fresh_indexed_sources"
    });
    Ok(())
}
