use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{BatchArgs, DoctorArgs, Request};
use serde_json::{Value, json};

#[path = "support/codegraph.rs"]
mod graph_fixture;

fn batch(app: &NativeApplication, jobs: Value) -> Value {
    let args: BatchArgs = serde_json::from_value(json!({"jobs":jobs})).unwrap();
    app.execute(Request::Batch(args))
}
fn job(path: &str) -> Value {
    json!({"tool":"harness_read","arguments":{"path":path,"start":1,"end":1}})
}

#[test]
fn ordered_results_preserve_partial_failures_and_normal_child_metrics() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "first").unwrap();
    std::fs::write(root.path().join("b.txt"), "second").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = batch(
        &app,
        json!([job("a.txt"), job("missing.txt"), job("b.txt")]),
    );
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(result["data"]["atomic"], false);
    let ids: Vec<_> = result["data"]["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["requestId"].as_str().unwrap())
        .collect();
    assert!(
        ids.iter()
            .all(|id| harness_domain::execution::valid_request_id(id))
    );
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[0], result["requestId"].as_str().unwrap());
    assert_eq!(result["data"]["results"][0]["data"]["text"], "first");
    assert_eq!(result["data"]["results"][1]["state"], "error");
    assert_eq!(result["data"]["results"][2]["data"]["text"], "second");
    let doctor = app.execute(Request::Doctor(DoctorArgs {
        versions: Some(false),
        connection: None,
        session: None,
    }));
    assert_eq!(
        doctor["data"]["metrics"]["byTool"]["harness_read"]["completed"],
        3
    );
    assert_eq!(
        doctor["data"]["metrics"]["byTool"]["harness_batch"]["completed"],
        1
    );
}

#[test]
fn validates_every_child_before_execution_and_refuses_mutations_or_recursion() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = batch(
        &app,
        // A path-only read is valid; use an invalid range to test prevalidation.
        json!([job("missing.txt"),{"tool":"harness_read","arguments":{"path":"x","start":0}}]),
    );
    assert_eq!(result["diagnostics"][0]["code"], "ARGUMENT");
    for tool in [
        "harness_replace",
        "harness_move",
        "harness_delete",
        "harness_proposal",
        "harness_verify",
        "harness_batch",
        "harness_checkpoint",
    ] {
        assert!(
            serde_json::from_value::<BatchArgs>(json!({"jobs":[{"tool":tool,"arguments":{}}]}))
                .is_err()
        );
    }
    let doctor = app.execute(Request::Doctor(DoctorArgs {
        versions: Some(false),
        connection: None,
        session: None,
    }));
    assert!(
        doctor["data"]["metrics"]["byTool"]
            .get("harness_read")
            .is_none()
    );
    assert_eq!(batch(&app, json!([]))["diagnostics"][0]["code"], "ARGUMENT");
    assert_eq!(
        batch(
            &app,
            json!([job("x"), job("x"), job("x"), job("x"), job("x")])
        )["diagnostics"][0]["code"],
        "ARGUMENT"
    );
}

#[test]
fn reports_all_child_validation_errors_without_starting_valid_children_or_echoing_values() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("exists.txt"), "must not be read").unwrap();
    let app = adapters::application(root.path()).unwrap();

    let result = batch(
        &app,
        json!([
            job("exists.txt"),
            {"tool":"harness_config","arguments":{"path":"SENSITIVE_VALUE_SENTINEL"}},
            {"tool":"harness_svg","arguments":{}}
        ]),
    );

    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(
        result["diagnostics"].as_array().unwrap().len(),
        2,
        "{result}"
    );
    let first = &result["diagnostics"][0];
    assert_eq!(first["code"], "ARGUMENT");
    assert_eq!(first["index"], 1);
    assert_eq!(first["tool"], "harness_config");
    assert_eq!(first["field"], "section");
    assert!(first["reason"].as_str().unwrap().contains("missing"));
    let second = &result["diagnostics"][1];
    assert_eq!(second["code"], "ARGUMENT");
    assert_eq!(second["index"], 2);
    assert_eq!(second["tool"], "harness_svg");
    assert_eq!(second["field"], "path");
    assert!(second["reason"].as_str().unwrap().contains("missing"));
    assert!(!result.to_string().contains("SENSITIVE_VALUE_SENTINEL"));

    let doctor = app.execute(Request::Doctor(DoctorArgs {
        versions: Some(false),
        connection: None,
        session: None,
    }));
    assert!(
        doctor["data"]["metrics"]["byTool"]
            .get("harness_read")
            .is_none(),
        "the valid first child must not have started: {doctor}"
    );
}

#[test]
fn accepts_args_alias_and_requested_concurrency_and_rejects_invalid_values() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "first").unwrap();
    let app = adapters::application(root.path()).unwrap();
    for concurrency in [1, 2, 3, 4] {
        let args: BatchArgs = serde_json::from_value(json!({
            "jobs": (0..4).map(|_| json!({"tool":"harness_read","args":{"path":"a.txt","start":1,"lines":1}})).collect::<Vec<_>>(),
            "maxConcurrent":concurrency
        })).unwrap();
        let result = app.execute(Request::Batch(args));
        assert_eq!(result["state"], "ok", "{result}");
        assert_eq!(result["data"]["maxConcurrentChildren"], concurrency);
        assert_eq!(result["data"]["results"].as_array().unwrap().len(), 4);
    }
    for concurrency in [0, 5] {
        let args: BatchArgs =
            serde_json::from_value(json!({"jobs":[job("a.txt")],"maxConcurrent":concurrency}))
                .unwrap();
        assert_eq!(
            app.execute(Request::Batch(args))["diagnostics"][0]["code"],
            "ARGUMENT"
        );
    }
    assert!(
        serde_json::from_value::<BatchArgs>(json!({"jobs":[{
            "tool":"harness_read","args":{},"arguments":{}
        }]}))
        .is_err()
    );
}

#[test]
fn oversized_aggregate_is_saved_in_batch_detail() {
    let root = tempfile::tempdir().unwrap();
    // Four children, each oversized: the aggregate exceeds the batch response
    // budget and per-child budgets, so each child keeps a truncated preview.
    // Per-line reads have their own budget, so use a bounded multi-line range.
    let line = "a".repeat(300);
    let source = (0..200).map(|_| line.clone()).collect::<Vec<_>>().join("\n");
    std::fs::write(root.path().join("a.txt"), source).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let job = json!({"tool":"harness_read","arguments":{"path":"a.txt","start":1,"end":200}});
    let jobs = (0..4).map(|_| job.clone()).collect::<Vec<_>>();
    let result = batch(&app, serde_json::Value::Array(jobs));
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["omitted"]["truncated"], true, "{result}");
    let first = &result["data"]["results"][0];
    assert_eq!(first["state"], "ok", "{result}");
    assert_eq!(first["data"]["previewTruncated"], true, "{result}");
    let excerpt = first["data"]["text"].as_str().unwrap_or_default();
    assert!(excerpt.chars().count() < 50_000, "{result}");
    assert!(excerpt.ends_with('…'), "{result}");
    assert!(result["detail"]["ref"].is_string(), "{result}");
    assert_eq!(result["detail"]["complete"], true, "{result}");
    assert_eq!(
        result["detail"]["nextCall"]["tool"],
        "harness_detail",
        "{result}"
    );
}

#[test]
fn explores_symbols_inside_batch_and_rejects_index_sync_before_any_child_runs() {
    let root = tempfile::tempdir().unwrap();
    drop(graph_fixture::fixture(root.path()));
    let app = adapters::application(root.path()).unwrap();
    let result = batch(
        &app,
        json!([
            {"tool":"harness_explore","arguments":{"filePath":"lib.rs","symbol":"target","view":"summary"}},
            job("lib.rs")
        ]),
    );
    assert_eq!(result["state"], "ok", "{result}");
    let children = result["data"]["results"].as_array().unwrap();
    assert_eq!(children[0]["tool"], "harness_explore");
    assert_eq!(children[0]["state"], "ok", "{result}");
    assert_eq!(children[0]["data"]["focus"]["symbol"], "target");
    assert_eq!(children[1]["tool"], "harness_read");
    assert_eq!(children[1]["state"], "ok", "{result}");

    let before_rejected = app.execute(Request::Doctor(DoctorArgs {
        versions: Some(false),
        connection: None,
        session: None,
    }));
    let reads_before = before_rejected["data"]["metrics"]["byTool"]["harness_read"]["completed"]
        .as_u64()
        .unwrap();
    let rejected = batch(
        &app,
        json!([
            job("lib.rs"),
            {"tool":"harness_explore","arguments":{"query":"target","syncIndex":true}}
        ]),
    );
    assert_eq!(rejected["state"], "error", "{rejected}");
    assert_eq!(rejected["diagnostics"][0]["index"], 1);
    assert_eq!(rejected["diagnostics"][0]["tool"], "harness_explore");
    assert_eq!(rejected["diagnostics"][0]["field"], "syncIndex");

    let doctor = app.execute(Request::Doctor(DoctorArgs {
        versions: Some(false),
        connection: None,
        session: None,
    }));
    assert_eq!(
        doctor["data"]["metrics"]["byTool"]["harness_read"]["completed"],
        reads_before
    );
}
