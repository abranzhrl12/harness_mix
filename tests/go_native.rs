use agent_harness::adapters::index_extract;
use harness_domain::execution::OperationContext;

#[test]
fn go_extraction_uses_utf16_columns_and_only_syntax_proven_edges() {
    let source = "package demo\nimport `fmt`\nvar _ = \"😀\"; func Café() {}\ntype Thing struct { Name string }\nfunc (t *Thing) Run() { fmt.Println(t.Name) }\n";
    let graph = index_extract::extract("thing.go", source, &OperationContext::default()).unwrap();
    assert_eq!(graph["nodes"][0]["kind"], "file");
    let nodes = graph["nodes"].as_array().unwrap();
    assert!(
        nodes
            .iter()
            .any(|n| n["name"] == "Thing" && n["kind"] == "struct")
    );
    assert!(
        nodes
            .iter()
            .any(|n| n["name"] == "Run" && n["kind"] == "method")
    );
    assert!(nodes.iter().any(|n| n["kind"] == "import"));
    assert!(
        nodes
            .iter()
            .any(|n| n["kind"] == "import" && n["name"] == "fmt")
    );
    assert!(
        graph["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "imports")
    );
    assert!(
        graph["edges"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["kind"] != "calls" && e["kind"] != "implements")
    );
    let function = nodes.iter().find(|n| n["name"] == "Café").unwrap();
    let at = source.find("func Café").unwrap();
    let prefix = source[..at].rsplit('\n').next().unwrap();
    assert_eq!(function["startColumn"], prefix.encode_utf16().count());
    assert!(
        prefix.len() != prefix.encode_utf16().count(),
        "fixture must distinguish UTF-8 and UTF-16 columns"
    );
}

#[test]
fn go_syntax_rejects_missing_package_and_malformed_declarations() {
    for source in ["", "func Foo() {}", "package demo\nfunc broken( {\n"] {
        assert_eq!(
            index_extract::extract("bad.go", source, &OperationContext::default())
                .unwrap_err()
                .0,
            "SYNTAX_ERROR"
        );
    }
}
