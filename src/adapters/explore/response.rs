//! Finalize source freshness at the original observation point and assemble responses.
use super::{graph::GraphWalk, selection::Selection, sources::LoadedSources};
use crate::adapters::project::Project;
use harness_domain::{
    Failure,
    contracts::{ExploreArgs, ExploreDirection},
    execution::OperationContext,
    results::{prefix, utf16_len},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) struct ResponseInput {
    pub(super) symbol: String,
    pub(super) impact: bool,
    pub(super) summary: bool,
    pub(super) direction: ExploreDirection,
    pub(super) max_depth: usize,
    pub(super) max_chars: usize,
    pub(super) max_files: usize,
    pub(super) version: i64,
    pub(super) selection: Selection,
    pub(super) graph: GraphWalk,
    pub(super) sources: LoadedSources,
}

pub(super) fn respond(
    project: &Project,
    args: &ExploreArgs,
    context: &OperationContext,
    assessment: super::super::index_freshness::Assessment,
    input: ResponseInput,
) -> Result<Value, Failure> {
    let ResponseInput {
        symbol,
        impact,
        summary,
        direction,
        max_depth,
        max_chars,
        max_files,
        version,
        selection,
        graph,
        sources,
    } = input;
    let Selection {
        candidates,
        candidates_omitted,
        selection_limited,
        files,
        targets,
        ..
    } = selection;
    let GraphWalk {
        nodes,
        edges,
        depths,
        expanded,
        depth_boundary,
        truncated: graph_truncated,
        per_node_edges_limited,
        total_edges_limited,
        ..
    } = graph;
    let LoadedSources {
        definitions,
        unresolved,
        text,
        context_omitted,
        truncated: source_truncated,
        definitions_limited,
    } = sources;
    let truncated = graph_truncated || source_truncated;
    let mut text = text;
    text.push_str(if summary {
        "\nRelaciones indexadas: ver graph.relations; grafo completo en detail.ref.\n"
    } else {
        "\n**Call paths / blast radius — indexed relationships**\n\nNot execution traces or exhaustive impact; current-source validity is reported in fileFreshness.\n"
    });
    let mut relation_keys = BTreeSet::new();
    let mut visible_edges: Vec<_> = edges
        .iter()
        .filter(|e| {
            relation_keys.insert((
                e["source"].to_string(),
                e["target"].to_string(),
                e["kind"].to_string(),
            ))
        })
        .collect();
    visible_edges.sort_by_key(|e| {
        (
            e["kind"] != "calls",
            e["source"].to_string(),
            e["target"].to_string(),
        )
    });
    if !summary {
        for selected in &targets {
            let callers: BTreeSet<_> = visible_edges
                .iter()
                .filter(|e| e["kind"] == "calls" && e["target"] == selected.id)
                .filter_map(|e| e["source"].as_str())
                .collect();
            let tests: BTreeSet<_> = callers
                .iter()
                .filter_map(|id| nodes.get(*id))
                .filter(|n| {
                    n.file_path
                        .split('/')
                        .any(|p| matches!(p, "tests" | "test"))
                        || n.name.starts_with("test_")
                })
                .map(|n| n.file_path.as_str())
                .collect();
            text.push_str(&format!(
                "- {}: {} unique direct callers in returned graph; test files: {}.\n",
                selected.name,
                callers.len(),
                if tests.is_empty() {
                    "none identified (not proof of absence)".to_owned()
                } else {
                    tests.into_iter().collect::<Vec<_>>().join(", ")
                }
            ));
        }
        text.push_str("\n**Relationships (calls first)**\n");
    }
    for edge in visible_edges.iter().take(if summary { 0 } else { 20 }) {
        let source = &nodes[edge["source"].as_str().ok_or(super::CONTRACT)?];
        let target = &nodes[edge["target"].as_str().ok_or(super::CONTRACT)?];
        text.push_str(&format!(
            "- {} ({}:{}) → {} ({}:{}) [{}]\n",
            source.name,
            source.file_path,
            source.start_line,
            target.name,
            target.file_path,
            target.start_line,
            edge["kind"].as_str().ok_or(super::CONTRACT)?
        ));
    }
    if !summary && visible_edges.len() > 20 {
        text.push_str(&format!(
            "\n> {} additional relationships omitted from text; available in graph/detail.\n",
            visible_edges.len() - 20
        ));
    }
    if !summary && (truncated || !unresolved.is_empty()) {
        text.push_str("\n> Partial context: inspect truncation and unresolved before drawing absence conclusions.\n");
    }
    let definition_hashes = definitions
        .iter()
        .filter_map(|d| {
            Some((
                d["path"].as_str()?.to_string(),
                d["hash"].as_str()?.to_string(),
            ))
        })
        .collect();
    let freshness = assessment.finish(project, &definition_hashes, context)?;
    let matching = freshness["status"] == "matches_observed_files";
    if args.require_matching_files == Some(true) && !matching {
        return Err(if freshness["status"] == "stale" {
            (
                "INDEX_STALE",
                "Contributing source files changed since indexing; synchronize the index before requiring matching files",
            )
        } else {
            (
                "INDEX_UNCHECKED",
                "Contributing source file matches could not be established within coverage and read budgets",
            )
        });
    }
    let diagnostics = if matching {
        vec![]
    } else {
        vec![
            json!({"code":if freshness["status"] == "stale" {"INDEX_STALE"} else {"INDEX_UNCHECKED"},"severity":"warning","message":"Contributing file freshness is stale or unverified; indexed relationships must not be treated as current source evidence"}),
        ]
    };
    let preview = prefix(&text, max_chars);
    let partial = truncated
        || context_omitted > 0
        || !unresolved.is_empty()
        || targets.is_empty()
        || !matching;
    let traversal = json!({"direction":direction,"maxDepth":max_depth,"nodeDepths":depths,"expandedNodes":expanded,"depthBoundaryReached":depth_boundary,
        "limits":{"edges":160,"nodes":176,"edgesPerNodeDirection":20},"semantics":"independent directed breadth-first traversals from selected exact-name targets; depths are minimum discovered hops within scanned edges per direction, not global shortest paths"});
    let graph_limited = selection_limited || per_node_edges_limited || total_edges_limited;
    let truncation = json!({"targetSelection":selection_limited,"depthBoundary":depth_boundary,"perNodeEdges":per_node_edges_limited,"totalEdges":total_edges_limited});
    let full_graph = json!({"format":"codegraph-sqlite-readonly-v1","schemaVersion":version,"files":files,"nodes":nodes.values().collect::<Vec<_>>(),"edges":edges,"truncated":graph_limited,"truncation":truncation,"traversal":traversal});
    let displayed_graph = if summary {
        super::presentation::summary_graph(
            &targets,
            &nodes,
            &edges,
            graph_limited,
            truncation.clone(),
            direction,
            max_depth,
        )?
    } else {
        full_graph.clone()
    };
    let context_truncation = json!({"definitionBodies":definitions_limited,"textPreview":utf16_len(&text)>max_chars,
        "relatedBodies":context_omitted>0,"summaryRelations":displayed_graph["summaryTruncated"]==true});
    let displayed_candidates: Vec<_> = candidates
        .iter()
        .take(if summary { 5 } else { 20 })
        .collect();
    let displayed_candidates_omitted =
        candidates_omitted + candidates.len() - displayed_candidates.len();
    let mut definitions = definitions;
    for definition in &mut definitions {
        let complete = definition["_bodyComplete"] == true
            && definition["_textEnd"]
                .as_u64()
                .is_some_and(|end| end <= max_chars as u64);
        definition.as_object_mut().unwrap().remove("_bodyComplete");
        definition.as_object_mut().unwrap().remove("_textEnd");
        definition["sourceStatus"] = json!(if complete {
            "complete"
        } else if summary {
            "signature_only"
        } else {
            "partial"
        });
        if !complete {
            let mut arguments = json!({"path":definition["path"],"symbol":definition["symbol"]["name"],
                "symbolId":definition["symbol"]["symbolId"],"view":"full"});
            if let Some(session) = &args.session {
                arguments["session"] = json!(session);
            }
            arguments["action"] = json!("read");
            definition["nextCall"] = json!({"tool":"harness_symbols","arguments":arguments});
        }
    }
    let requested = super::query_symbols(args);
    Ok(
        json!({"text":preview,"focus":{"symbol":symbol,"status":if definitions.iter().any(|d| d["role"] == "target") || (super::razor_file_selector(args) && !targets.is_empty()) {"resolved"} else {"unresolved"}},
        "view":if impact {"impact"} else if summary {"summary"} else {"full"},"definitionView":if summary {"signature"} else {"full"},"editReady":false,"contextTruncation":context_truncation,
        "reuseGuidance":"Use complete definitions already present in your context; do not read them again unless missing or changed. For partial/signature_only definitions use nextCall only when the missing body is needed. Hashes identify the observed source, not future freshness. Numbered Markdown is not replacement text.",
        "contextSelection":{"requestedSymbols":requested,"relatedReturned":definitions.iter().filter(|d| d["role"] == "related").count(),"relatedOmitted":context_omitted,"maxRelatedSymbols":4,"maxFiles":max_files,"policy":"direct calls and local SQL table dependencies; production before tests; ambiguous local bodies are not guessed"},
        "definitions":definitions,"unresolved":unresolved,"candidates":displayed_candidates,"candidatesOmitted":displayed_candidates_omitted,"selection":{"filePath":args.file_path,"container":args.container,"symbolId":args.symbol_id,"multipleTargets":targets.len()>1,"symbolIdNamespace":"codegraph-node-id"},"graph":displayed_graph,"relationshipSource":"codegraph-index",
        "indexFreshness":"not_certified","synchronized":false,"absenceProven":false,"fileFreshness":freshness,"diagnostics":diagnostics,
        "synchronization":{"performed":false,"nativeIndexerAvailable":true,"supportedLanguages":["rust","csharp","javascript","typescript","jsx","tsx"],"reason":"syncIndex_not_requested"},
        "notice":"Bounded indexed relationships, not execution traces. Local symbol readers support Rust, JavaScript and TypeScript; other bodies may be unresolved. graph.truncated concerns graph selection/edges only; contextTruncation concerns presentation. No exhaustive impact, absence or index freshness claims. Indexer availability does not grant write authorization. Preview is not replacement text.",
        "truncated":truncated || utf16_len(&text)>max_chars || context_omitted>0 || displayed_graph["summaryTruncated"]==true,"_complete":!partial,
        "_detail":{"text":text,"definitionView":if summary {"signature"} else {"full"},"graph":full_graph,"contextTruncation":context_truncation,"candidates":candidates,"candidatesOmitted":candidates_omitted,"definitions":definitions,"unresolved":unresolved,"fileFreshness":freshness,"indexFreshness":"not_certified","synchronized":false}}),
    )
}
