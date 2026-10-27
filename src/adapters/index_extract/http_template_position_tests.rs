use super::http_razor::enrich;
use harness_domain::execution::OperationContext;
use serde_json::json;

#[test]
fn first_line_scripts_keep_utf16_columns_and_same_line_function_callers() {
    let source = "<span>🧪</span><script>function one(){fetch('/one');} function two(){fetch('/two');}</script>";
    let mut graph = json!({"nodes":[{
        "id":"file","kind":"file","filePath":"page.html","language":"html",
        "startLine":1,"startColumn":0,"endLine":1,"endColumn":source.encode_utf16().count()
    }],"edges":[],"unresolvedReferences":[]});
    enrich("page.html", source, &mut graph, &OperationContext::default()).unwrap();
    let refs = graph["unresolvedReferences"].as_array().unwrap();
    assert_eq!(refs.len(), 2, "{graph}");
    for (reference, name, marker) in [
        (&refs[0], "one", "fetch('/one')"),
        (&refs[1], "two", "fetch('/two')"),
    ] {
        let at = source.find(marker).unwrap();
        assert_eq!(reference["line"], 1);
        assert_eq!(reference["column"], source[..at].encode_utf16().count());
        assert!(graph["nodes"].as_array().unwrap().iter().any(|node|
            node["id"] == reference["fromNodeId"] && node["name"] == name), "{graph}");
    }
}
