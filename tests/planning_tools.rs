use agent_harness::adapters;
use harness_domain::{
    contracts::{ReplaceArgs, Request},
    hash,
};
use serde_json::{Value, json};
#[path = "support/codegraph.rs"]
mod graph_fixture;

#[test]
fn impact_reuses_inbound_graph_and_preserves_freshness_rejection() {
    let root = tempfile::tempdir().unwrap();
    drop(graph_fixture::fixture(root.path()));
    let app = adapters::application(root.path())
        .unwrap()
        .with_tool_profile(harness_domain::tool_contract::ToolProfile::Investigate);
    let query = || {
        Request::Impact(
            serde_json::from_value(json!({
                "file":"lib.rs","symbol":"target","requireMatchingFiles":true
            }))
            .unwrap(),
        )
    };
    let result = app.execute(query());
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["counts"]["directCallers"], 1);
    assert_eq!(result["data"]["callers"][0]["symbol"], "caller");
    assert_eq!(result["data"]["callers"][0]["declarationLine"], 2);
    std::fs::write(root.path().join("lib.rs"), "pub fn target() {}\n").unwrap();
    let stale = app.execute(query());
    assert_eq!(stale["diagnostics"][0]["code"], "INDEX_STALE", "{stale}");
}

fn preview(app: &adapters::NativeApplication, original: &str, new: &str) -> Value {
    let args: ReplaceArgs =
        serde_json::from_value(json!({"path":"sample.rs","hash":hash(original.as_bytes()),
        "oldText":"1","newText":new}))
        .unwrap();
    app.execute(Request::PatchPreview(args))
}

#[test]
fn preview_never_publishes_and_rejects_stale_sources() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("sample.rs");
    let before = "fn sample() { let x = 1; }\r\n";
    std::fs::write(&file, before).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = preview(&app, before, "2");
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["published"], false);
    assert_eq!(result["data"]["syntax"]["status"], "valid");
    assert_eq!(
        result["data"]["candidateHash"],
        hash(before.replace('1', "2").as_bytes())
    );
    assert!(
        result["data"]["patch"]
            .as_str()
            .unwrap()
            .contains("+fn sample() { let x = 2; }\r\n")
    );
    let invalid = preview(&app, before, "{");
    assert_eq!(invalid["data"]["syntax"]["status"], "error", "{invalid}");
    assert!(!invalid["data"]["patch"].as_str().unwrap().is_empty());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    std::fs::write(&file, "fn changed() {}\n").unwrap();
    let stale = preview(&app, before, "2");
    assert_eq!(stale["diagnostics"][0]["code"], "STALE", "{stale}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "fn changed() {}\n");
}

#[test]
fn impact_projection_counts_unique_callers_and_exposes_coverage() {
    use harness_domain::assistance::{ImpactArgs, impact};
    let args: ImpactArgs =
        serde_json::from_value(json!({"file":"auth.rs","symbol":"validate","limit":1})).unwrap();
    let graph = json!({"nodes":[
        {"id":"t","name":"validate","filePath":"auth.rs","startLine":2},
        {"id":"a","name":"route","filePath":"routes.rs","startLine":40},
        {"id":"b","name":"outer","filePath":"main.rs","startLine":10}],
        "edges":[{"source":"a","target":"t","kind":"calls"},{"source":"a","target":"t","kind":"calls"},
            {"source":"b","target":"a","kind":"calls"}],"truncated":true});
    let r = impact(
        &args,
        json!({"graph":{"targetIds":["t"]},"selection":{"multipleTargets":false},
        "_detail":{"graph":graph},"fileFreshness":{"status":"stale"}}),
    );
    assert_eq!(r["counts"]["callers"], 2);
    assert_eq!(r["counts"]["directCallers"], 1);
    assert_eq!(r["callers"][0]["declarationLine"], 40);
    assert_eq!(r["callersOmitted"], 1);
    assert_eq!(r["fileFreshness"]["status"], "stale");
    assert_eq!(r["absenceProven"], false);
    assert_eq!(r["truncated"], true);
    assert_eq!(r["_detail"]["callers"].as_array().unwrap().len(), 2);
}
