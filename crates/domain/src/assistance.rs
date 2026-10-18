//! Read-only planning contracts and compact projections of indexed impact.
use crate::contracts::{ExploreArgs, ExploreDirection, ExploreView};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ImpactArgs {
    #[serde(alias = "filePath")]
    pub file: String,
    pub symbol: String,
    pub container: Option<String>,
    pub symbol_id: Option<String>,
    /// Indexed inbound hops, 1..4; defaults to 1.
    pub max_depth: Option<usize>,
    /// Maximum displayed callers, 1..100; detail retains the bounded result.
    pub limit: Option<usize>,
    pub require_matching_files: Option<bool>,
    pub session: Option<String>,
}
impl ImpactArgs {
    pub fn exploration(&self) -> Result<ExploreArgs, crate::Failure> {
        if !(1..=100).contains(&self.limit.unwrap_or(20)) {
            return Err(("ARGUMENT", "limit must be between 1 and 100"));
        }
        Ok(ExploreArgs {
            query: self.symbol.clone(),
            symbol: Some(self.symbol.clone()),
            file_path: Some(self.file.clone()),
            container: self.container.clone(),
            symbol_id: self.symbol_id.clone(),
            view: Some(ExploreView::Summary),
            direction: Some(ExploreDirection::Inbound),
            max_depth: self.max_depth,
            max_files: Some(1),
            max_chars: Some(1000),
            sync_index: Some(false),
            require_matching_files: self.require_matching_files,
            session: self.session.clone(),
        })
    }
}

/// Counts concern only the bounded graph. Node locations are declarations, not call sites.
pub fn impact(args: &ImpactArgs, result: Value) -> Value {
    let graph = &result["_detail"]["graph"];
    let targets: BTreeSet<&str> = result["graph"]["targetIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let nodes: BTreeMap<&str, &Value> = graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();
    let mut callers = BTreeMap::<&str, Value>::new();
    let mut kinds = BTreeMap::<String, usize>::new();
    for edge in graph["edges"].as_array().into_iter().flatten() {
        *kinds
            .entry(edge["kind"].as_str().unwrap_or("unknown").into())
            .or_default() += 1;
        if edge["kind"] != "calls" {
            continue;
        }
        let Some(id) = edge["source"].as_str() else {
            continue;
        };
        let Some(node) = nodes.get(id) else { continue };
        let direct = edge["target"]
            .as_str()
            .is_some_and(|id| targets.contains(id));
        let entry = callers.entry(id).or_insert_with(|| {
            json!({
                "symbolId":id,"symbol":node["name"],"file":node["filePath"],
                "declarationLine":node["startLine"],"direct":false,"recursive":false
            })
        });
        if direct {
            entry["direct"] = json!(true);
        }
        if edge["source"] == edge["target"] {
            entry["recursive"] = json!(true);
        }
    }
    let mut all: Vec<_> = callers.into_values().collect();
    all.sort_by_key(|n| {
        (
            n["direct"] != true,
            n["file"].to_string(),
            n["declarationLine"].as_i64(),
        )
    });
    let files: BTreeSet<_> = all.iter().filter_map(|n| n["file"].as_str()).collect();
    let limit = args.limit.unwrap_or(20);
    let omitted = all.len().saturating_sub(limit);
    let graph_limited = graph["truncated"] == true;
    let ambiguous = result["selection"]["multipleTargets"] == true;
    json!({"file":args.file,"symbol":args.symbol,"focus":result["focus"],"files":files,
        "callers":all.iter().take(limit).collect::<Vec<_>>(),"callersOmitted":omitted,
        "counts":{"callers":all.len(),"directCallers":all.iter().filter(|n|n["direct"]==true).count(),"files":files.len(),"scope":"returned-subgraph-not-global"},
        "relationshipKinds":kinds,"locationKind":"caller-declaration-not-call-site",
        "ambiguous":ambiguous,"candidates":result["candidates"],"candidatesOmitted":result["candidatesOmitted"],
        "fileFreshness":result["fileFreshness"],"diagnostics":result["diagnostics"],
        "unresolved":result["unresolved"],"traversal":graph["traversal"],"truncation":graph["truncation"],
        "truncated":graph_limited || omitted>0,"absenceProven":false,"indexFreshness":"not_certified",
        "notice":"Indexed callers only; dynamic dispatch, macros and unresolved references may be absent. Declaration lines are not exact call sites. Inspect ambiguity and coverage before editing.",
        "_complete":result["_complete"]==true && !ambiguous && omitted==0,
        "_detail":{"callers":all,"graph":graph,"exploration":result}})
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ObserveArgs {
    #[serde(alias = "job_id")]
    pub task_id: String,
    /// Rust regex; omitted matches every retained line. Maximum 1024 bytes.
    pub filter: Option<String>,
    /// Maximum matching lines, 1..200; defaults to 20. Context is additional.
    pub lines: Option<usize>,
    /// Context lines on each side, 0..20; defaults to 2.
    pub before: Option<usize>,
    pub after: Option<usize>,
    /// stdout, stderr or both (default).
    pub stream: Option<String>,
    /// Only output events after this cursor. Context is limited to that window.
    pub cursor: Option<u64>,
    /// UTF-8 byte ceiling, 256..16000; defaults to 6000.
    pub max_chars: Option<usize>,
}
