#[test]
fn numbers_current_source_without_changing_line_content() {
    let rendered = super::source("a.rs", 7, "fn a() {\r\n    b();\r\n}", false);
    assert!(rendered.contains("```rust\n7 | fn a() {\n8 |     b();\n9 | }"));
    assert!(!rendered.contains("truncated"));
}

#[test]
fn source_cannot_close_its_fence_and_truncation_is_explicit() {
    let rendered = super::source("a.rs", 1, "```\nhello", true);
    assert!(rendered.contains("\n````rust\n1 | ```"));
    assert!(rendered.contains("Source fragment truncated"));
}

#[test]
fn unknown_extensions_keep_a_bare_fence() {
    let rendered = super::source("notes.txt", 1, "yes", false);
    assert!(rendered.contains("```\n1 | yes"));
}

#[test]
fn summary_keeps_calls_ahead_of_type_references_and_reports_omissions() {
    use super::super::model::Node;
    use harness_domain::contracts::ExploreDirection;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    let focus = Node {
        id: "focus".into(),
        name: "explore".into(),
        kind: "function".into(),
        file_path: "src/explore.rs".into(),
        start_line: 10,
    };
    let mut nodes = BTreeMap::from([("focus".to_owned(), focus.clone())]);
    let mut edges = Vec::<Value>::new();
    for index in 0..5 {
        let id = format!("type-{index}");
        let node = Node {
            id: id.clone(),
            name: format!("Type{index}"),
            kind: "struct".into(),
            file_path: format!("src/types{index}.rs"),
            start_line: (index + 1) as i64,
        };
        nodes.insert(id.clone(), node);
        edges.push(json!({
            "id": index + 1,
            "source": "focus",
            "target": id,
            "kind": "references"
        }));
    }
    let caller = Node {
        id: "caller".into(),
        name: "run".into(),
        kind: "function".into(),
        file_path: "src/caller.rs".into(),
        start_line: 3,
    };
    nodes.insert(caller.id.clone(), caller.clone());
    // Put the important edge last to reproduce storage order hiding it.
    edges.push(json!({
        "id": 6,
        "source": focus.id,
        "target": caller.id,
        "kind": "calls",
        "resolutionEvidence": {
            "provenance": "heuristic",
            "resolvedBy": "ra-module-subset-v1",
            "storedConfidence": 0.9
        }
    }));

    let summary = super::summary_graph(
        std::slice::from_ref(&focus),
        &nodes,
        &edges,
        false,
        json!({"targetSelection":false,"depthBoundary":false,"perNodeEdges":false,"totalEdges":false}),
        ExploreDirection::Both,
        1,
    )
    .unwrap();
    let relations = summary["relations"].as_array().unwrap();
    assert_eq!(relations.len(), 4);
    let call = relations.iter().find(|edge| edge["kind"] == "calls").unwrap();
    assert_eq!(call["resolutionEvidence"]["provenance"], "heuristic");
    assert_eq!(call["resolutionEvidence"]["resolvedBy"], "ra-module-subset-v1");
    assert_eq!(call["resolutionEvidence"]["storedConfidence"], 0.9);
    assert_eq!(summary["resolutionEvidence"]["recordedEdges"], 1);
    assert_eq!(summary["resolutionEvidence"]["unrecordedEdges"], 5);
    assert_eq!(summary["resolutionEvidence"]["validated"], false);
    assert_eq!(summary["relationsOmitted"], 2);
    assert_eq!(summary["summaryTruncated"], true);

    let mut reversed_edges = edges.clone();
    reversed_edges.reverse();
    let reordered = super::summary_graph(
        std::slice::from_ref(&focus),
        &nodes,
        &reversed_edges,
        false,
        json!({"targetSelection":false,"depthBoundary":false,"perNodeEdges":false,"totalEdges":false}),
        ExploreDirection::Both,
        1,
    )
    .unwrap();
    assert_eq!(summary, reordered);
}
