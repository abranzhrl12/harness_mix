use super::*;

#[test]
fn extraction_model_projects_typed_rows_without_changing_optional_json_values() {
    let extraction = json!({
        "nodes":[{
            "id":"n1",
            "kind":"function",
            "name":"run",
            "qualifiedName":"module::run",
            "isExported":true,
            "decorators":["test"],
            "typeParameters":["T"]
        }],
        "edges":[{"source":"n1","target":"n2","kind":"calls","metadata":{"source":"native"}}],
        "unresolvedReferences":[{"fromNodeId":"n1","referenceName":"missing","referenceKind":"calls","line":7,"column":2,"candidates":[]}],
        "coverage":{"complete":false}
    });
    let graph = ExtractedGraph::prepare(&extraction, &OperationContext::default()).unwrap();

    assert_eq!(graph.nodes[0].qualified_name, "module::run");
    assert_eq!(graph.nodes[0].is_exported, true);
    assert_eq!(
        graph.nodes[0].decorators.unwrap().to_string(),
        r#"["test"]"#
    );
    assert_eq!(graph.edges[0].metadata.unwrap()["source"], "native");
    assert_eq!(graph.unresolved_references[0].line, 7);
    assert_eq!(
        graph.unresolved_references[0]
            .candidates
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(graph.coverage.unwrap()["complete"], false);
}

#[test]
fn duplicate_node_ids_with_same_identity_are_merged_with_combined_source_range() {
    let extraction = json!({
        "nodes":[
            {"id":"n1","kind":"function","name":"run","qualifiedName":"module::run","startLine":3,"endLine":3},
            {"id":"n1","kind":"function","name":"run","qualifiedName":"module::run","startLine":1,"endLine":5}
        ],
        "edges":[],
        "unresolvedReferences":[]
    });
    let graph = ExtractedGraph::prepare(&extraction, &OperationContext::default()).unwrap();
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.duplicate_nodes, 1);
    assert_eq!(graph.nodes[0].start_line, Some(1));
    assert_eq!(graph.nodes[0].end_line, Some(5));
}

#[test]
fn conflicting_duplicate_node_ids_remain_a_contract_error() {
    let extraction = json!({
        "nodes":[
            {"id":"n1","kind":"function","name":"run","qualifiedName":"module::run"},
            {"id":"n1","kind":"class","name":"run","qualifiedName":"module::run"}
        ],
        "edges":[],
        "unresolvedReferences":[]
    });
    assert_eq!(
        ExtractedGraph::prepare(&extraction, &OperationContext::default()).err(),
        Some(super::super::INDEX)
    );
}

#[test]
fn duplicate_nodes_on_same_line_merge_column_bounds() {
    let extraction = json!({
        "nodes":[
            {"id":"n1","kind":"variable","name":"value","qualifiedName":"module::value","startLine":3,"startColumn":12,"endLine":3,"endColumn":20},
            {"id":"n1","kind":"variable","name":"value","qualifiedName":"module::value","startLine":3,"startColumn":4,"endLine":3,"endColumn":28}
        ],
        "edges":[],
        "unresolvedReferences":[]
    });
    let graph = ExtractedGraph::prepare(&extraction, &OperationContext::default()).unwrap();
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.nodes[0].start_line, Some(3));
    assert_eq!(graph.nodes[0].start_column, Some(4));
    assert_eq!(graph.nodes[0].end_line, Some(3));
    assert_eq!(graph.nodes[0].end_column, Some(28));
}

#[test]
fn duplicate_method_ids_with_different_overload_signatures_are_rejected() {
    let extraction = json!({
        "nodes":[
            {"id":"n1","kind":"method","name":"run","qualifiedName":"module::run","signature":"void run()"},
            {"id":"n1","kind":"method","name":"run","qualifiedName":"module::run","signature":"void run(int value)"}
        ],
        "edges":[],
        "unresolvedReferences":[]
    });
    assert_eq!(
        ExtractedGraph::prepare(&extraction, &OperationContext::default()).err(),
        Some(super::super::INDEX)
    );
}

#[test]
fn borrowed_required_text_preserves_original_index_validation_limits() {
    assert_eq!(
        required_str(&json!({"id":""}), "id"),
        Err(super::super::INDEX)
    );
    let too_long = "x".repeat(65_537);
    assert_eq!(
        required_str(&json!({"id":too_long}), "id"),
        Err(super::super::INDEX)
    );
}

#[test]
fn extracted_reference_still_requires_both_coordinates() {
    let extraction = json!({
        "nodes":[],
        "edges":[],
        "unresolvedReferences":[{
            "fromNodeId":"n1",
            "referenceName":"missing",
            "referenceKind":"calls",
            "line":7
        }]
    });
    assert_eq!(
        ExtractedGraph::prepare(&extraction, &OperationContext::default()).err(),
        Some(super::super::INDEX)
    );
}

#[test]
fn report_presentation_keeps_public_flags_and_optional_coverage() {
    let empty = PublicationReport::unchanged("a.rs");
    assert_eq!(empty["changed"], false);
    assert_eq!(empty["extractionPerformed"], false);
    assert_eq!(empty["committed"], false);
    assert!(empty.get("extractionCoverage").is_none());

    let report = PublicationReport {
        path: "a.rs".into(),
        changed: true,
        extraction_performed: true,
        deleted: false,
        nodes_written: 1,
        incoming_edges_retained: 2,
        incoming_references_recovered: 3,
        provider_nodes_preserved: 4,
        provider_edges_retained: 5,
        provider_edges_dropped: 6,
        import_references_rechecked: 7,
        framework_references_rechecked: 8,
        razor_references_rechecked: 9,
        references_attempted: 10,
        references_resolved: 6,
        extraction_coverage: Some(json!({"complete":false})),
    }
    .into_json();
    assert_eq!(report["referencesPending"], 4);
    assert_eq!(report["committed"], false);
    assert_eq!(report["synchronized"], false);
    assert_eq!(report["extractionCoverage"]["complete"], false);
}
