//! Pure source formatting, context selection and graph summaries.
use super::model::Node;
use harness_domain::{Failure, contracts::ExploreDirection};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn header(symbol: &str) -> String {
    format!(
        "**Exploration: {}**\n\nIndexed relationships; freshness is not certified.\n\n**Source Code**\n",
        symbol.replace(['\r', '\n'], " ")
    )
}

pub(super) fn source(path: &str, start: u64, body: &str, truncated: bool) -> String {
    // Pick a fence longer than any backtick run in untrusted source.
    let fence = "`".repeat(
        body.split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1,
    );
    let numbered = body
        .lines()
        .enumerate()
        .map(|(offset, line)| format!("{}\t{line}", start + offset as u64))
        .collect::<Vec<_>>()
        .join("\n");
    let path = path.replace(['\r', '\n', '`'], "_");
    let notice = if truncated {
        "\n> Source fragment truncated; use read_symbol for exact code.\n"
    } else {
        ""
    };
    format!("\n### {path}:{start}\n\n{fence}\n{numbered}\n{fence}\n{notice}")
}

// Selection only: deterministic, bounded, and independent of source rendering.
pub(super) fn context_nodes<'a>(
    targets: &[Node],
    nodes: &'a BTreeMap<String, Node>,
    edges: &[Value],
    max_files: usize,
) -> (Vec<&'a Node>, usize) {
    let ids: BTreeSet<_> = targets.iter().map(|n| n.id.as_str()).collect();
    let mut candidates = BTreeMap::new();
    for edge in edges.iter().filter(|e| e["kind"] == "calls") {
        let source = edge["source"].as_str().unwrap_or("");
        let target = edge["target"].as_str().unwrap_or("");
        let related = if ids.contains(source) {
            Some((target, 0))
        } else if ids.contains(target) {
            Some((source, 1))
        } else {
            None
        };
        if let Some((id, direction)) = related {
            if !ids.contains(id) {
                if let Some(node) = nodes.get(id) {
                    let test = node
                        .file_path
                        .split('/')
                        .any(|p| matches!(p, "tests" | "test"))
                        || node.name.starts_with("test_");
                    candidates
                        .entry((
                            test,
                            direction,
                            node.file_path.as_str(),
                            node.start_line,
                            id,
                        ))
                        .or_insert(node);
                }
            }
        }
    }
    // A local SQL call has a useful second step: the tables read/written by its
    // procedure. Include that evidence under the same existing context budget.
    let procedures: BTreeSet<_> = targets
        .iter()
        .chain(candidates.values().copied())
        .filter(|node| node.kind == "stored_procedure" && node.id.starts_with("sql-local:"))
        .map(|node| node.id.as_str())
        .collect();
    for edge in edges.iter().filter(|edge| edge["kind"] == "references") {
        if !procedures.contains(edge["source"].as_str().unwrap_or("")) {
            continue;
        }
        let id = edge["target"].as_str().unwrap_or("");
        if let Some(node) = nodes.get(id) {
            if node.kind == "table" && node.id.starts_with("sql-local:") && !ids.contains(id) {
                candidates
                    .entry((false, 2, node.file_path.as_str(), node.start_line, id))
                    .or_insert(node);
            }
        }
    }
    let mut files: BTreeSet<_> = targets.iter().map(|n| n.file_path.as_str()).collect();
    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    // Round-robin files prevents local guards/helpers from consuming every slot
    // before a cross-file part of the flow. Production still precedes tests.
    let mut per_file = BTreeMap::<&str, usize>::new();
    for test_pass in [false, true] {
        for quota in 1..=4 {
            for ((is_test, _, _, _, _), node) in &candidates {
                if *is_test != test_pass || seen.contains(&node.id) || selected.len() >= 4 {
                    continue;
                }
                if per_file.get(node.file_path.as_str()).copied().unwrap_or(0) >= quota {
                    continue;
                }
                if !files.contains(node.file_path.as_str()) && files.len() >= max_files {
                    continue;
                }
                files.insert(node.file_path.as_str());
                *per_file.entry(node.file_path.as_str()).or_default() += 1;
                seen.insert(node.id.clone());
                selected.push(*node);
            }
        }
    }
    let omitted = candidates
        .values()
        .map(|n| &n.id)
        .collect::<BTreeSet<_>>()
        .len()
        .saturating_sub(selected.len());
    (selected, omitted)
}

pub(super) fn summary_graph(
    targets: &[Node],
    nodes: &BTreeMap<String, Node>,
    edges: &[Value],
    graph_limited: bool,
    truncation: Value,
    direction: ExploreDirection,
    max_depth: usize,
) -> Result<Value, Failure> {
    let mut kinds = BTreeMap::<String, usize>::new();
    for edge in edges {
        *kinds
            .entry(edge["kind"].as_str().unwrap_or("unknown").into())
            .or_default() += 1;
    }
    let mut relations = Vec::new();
    let mut shown = BTreeSet::new();
    for incoming in [true, false] {
        for edge in edges
            .iter()
            .filter(|edge| {
                targets.iter().any(|n| {
                    if incoming {
                        edge["target"] == n.id
                    } else {
                        edge["source"] == n.id
                    }
                })
            })
            .take(4)
        {
            if !shown.insert(edge["id"].as_i64().ok_or(super::CONTRACT)?) {
                continue;
            }
            let source = &nodes[edge["source"].as_str().ok_or(super::CONTRACT)?];
            let target = &nodes[edge["target"].as_str().ok_or(super::CONTRACT)?];
            relations.push(json!({"source":{"name":source.name,"filePath":source.file_path,"line":source.start_line},"target":{"name":target.name,"filePath":target.file_path,"line":target.start_line},"kind":edge["kind"]}));
        }
    }
    let omitted_relations = edges.len().saturating_sub(relations.len());
    Ok(
        json!({"format":"codegraph-summary-v1","countsScope":"returned-subgraph-not-global", "nodeCount":nodes.len(),"edgeCount":edges.len(),"edgeKinds":kinds,
        "relations":relations,"relationsOmitted":omitted_relations,"relationsOmittedScope":"summary_display_only","summaryTruncated":omitted_relations>0,
        "targetIds":targets.iter().map(|n| &n.id).collect::<Vec<_>>(),"truncated":graph_limited,"truncation":truncation,
        "direction":direction,"maxDepth":max_depth,"fullGraph":"detail.ref -> graph; use data_query resultRef for projection"}),
    )
}

#[cfg(test)]
mod tests;
