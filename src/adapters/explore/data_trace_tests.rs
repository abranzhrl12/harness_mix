use super::*;
use serde_json::json;

#[test]
fn rust_file_selector_does_not_search_unrelated_javascript_for_lineage() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("app.js"), "function main() { return main; }").unwrap();
    let project = Project::new(root.path()).unwrap();
    let args: ExploreArgs = serde_json::from_value(json!({"filePath":"src/main.rs","symbol":"main"})).unwrap();
    let result = trace(&project, &args, "main", &[], &BTreeSet::new(), &OperationContext::default()).unwrap();
    assert!(result.is_none());
}

#[test]
fn oversized_source_is_reported_as_incomplete() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("large.js"), "x".repeat(2 * 1024 * 1024 + 1)).unwrap();
    let project = Project::new(root.path()).unwrap();
    let args: ExploreArgs = serde_json::from_value(json!({"filePath":"large.js","symbol":"ItemsAnticipo"})).unwrap();
    let trace = trace(&project, &args, "ItemsAnticipo", &[], &BTreeSet::new(), &OperationContext::default())
        .unwrap().unwrap();
    assert_eq!(serde_json::to_value(trace).unwrap()["truncated"], true);
}

#[test]
fn trace_finds_property_in_large_explicit_javascript_file() {
    let root = tempfile::tempdir().unwrap();
    let source = format!("//{}\ndata.ItemsAnticipo = ItemsAnticipos;\n", "x".repeat(600 * 1024));
    std::fs::write(root.path().join("large.js"), source).unwrap();
    let project = Project::new(root.path()).unwrap();
    let args: ExploreArgs = serde_json::from_value(json!({"filePath":"large.js","symbol":"ItemsAnticipo"})).unwrap();
    let trace = trace(&project, &args, "ItemsAnticipo", &[], &BTreeSet::new(), &OperationContext::default())
        .unwrap().unwrap();
    let value = serde_json::to_value(trace).unwrap();
    assert!(value["layers"].as_array().unwrap().iter().any(|layer|
        layer["layer"] == "json" && layer["matches"].as_array().unwrap().iter()
            .any(|m| m["filePath"] == "large.js" && m["line"] == 2)), "{value}");
}
