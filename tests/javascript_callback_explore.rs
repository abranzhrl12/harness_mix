#![cfg(windows)]
use agent_harness::adapters::{application, index_store::{initialize, sync_file}};
use harness_domain::{contracts::Request, execution::OperationContext};
use serde_json::json;

#[test]
fn explore_reports_ambiguous_local_definitions_without_fabricating_source() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    std::fs::write(root.path().join("callbacks.js"),
        "a(function(){ function same(){return 1;} }); b(function(){ function same(){return 2;} });").unwrap();
    sync_file(root.path(), "callbacks.js", &context).unwrap();
    let app = application(root.path()).unwrap();
    let args = serde_json::from_value(json!({"filePath":"callbacks.js","symbol":"same","view":"summary"})).unwrap();
    let result = app.execute(Request::Explore(args));
    assert!(result["data"]["unresolved"].as_array().unwrap().iter()
        .any(|item| item["code"] == "AMBIGUOUS_SYMBOL"), "{result}");
    assert!(result["data"]["definitions"].as_array().is_none_or(|items| items.is_empty()), "{result}");
}

#[test]
fn explore_traces_property_in_large_indexed_file() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    let source = format!("//{}\nfunction process(data) {{ data.ItemsAnticipo = []; }}\n", "x".repeat(600 * 1024));
    std::fs::write(root.path().join("large.js"), source).unwrap();
    sync_file(root.path(), "large.js", &context).unwrap();
    let app = application(root.path()).unwrap();
    let args = serde_json::from_value(json!({"query":"ItemsAnticipo","view":"summary"})).unwrap();
    let result = app.execute(Request::Explore(args));
    assert_eq!(result["state"], "ok", "{result}");
    assert!(result["data"]["dataTrace"]["layers"].as_array().unwrap().iter().any(|layer|
        layer["layer"] == "json" && layer["matches"].as_array().unwrap().iter()
            .any(|m| m["filePath"] == "large.js")), "{result}");
}

#[test]
fn explore_reads_indexed_function_from_ready_callback_in_large_file() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    let source = format!("//{}\n$(document).ready(function () {{ function DOrden_BuscarporId(id) {{ return id; }} }});\n", "x".repeat(600 * 1024));
    std::fs::write(root.path().join("large.js"), source).unwrap();
    sync_file(root.path(), "large.js", &context).unwrap();
    let app = application(root.path()).unwrap();
    let args = serde_json::from_value(json!({"filePath":"large.js","symbol":"DOrden_BuscarporId","view":"summary"})).unwrap();
    let result = app.execute(Request::Explore(args));
    assert_eq!(result["state"], "ok", "{result}");
    assert!(result["data"]["definitions"].as_array().unwrap().iter().any(|d|
        d["symbol"]["name"] == "DOrden_BuscarporId"), "{result}");
    assert!(result["data"]["unresolved"].as_array().is_none_or(|items| items.is_empty()), "{result}");
}
