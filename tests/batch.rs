use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{BatchArgs, DoctorArgs, Request};
use serde_json::{Value, json};

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
        json!([job("missing.txt"),{"tool":"harness_read","arguments":{"path":"x"}}]),
    );
    assert_eq!(result["diagnostics"][0]["code"], "ARGUMENT");
    for tool in [
        "harness_replace",
        "harness_move",
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
fn accepts_args_alias_and_requested_concurrency_and_rejects_invalid_values() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "first").unwrap();
    let app = adapters::application(root.path()).unwrap();
    for concurrency in [2, 3, 4] {
        let args: BatchArgs = serde_json::from_value(json!({
            "jobs": (0..4).map(|_| json!({"tool":"harness_read","args":{"path":"a.txt","start":1,"lines":1}})).collect::<Vec<_>>(),
            "maxConcurrent":concurrency
        })).unwrap();
        let result = app.execute(Request::Batch(args));
        assert_eq!(result["state"], "ok", "{result}");
        assert_eq!(result["data"]["maxConcurrentChildren"], concurrency);
        assert_eq!(result["data"]["results"].as_array().unwrap().len(), 4);
    }
    for concurrency in [0, 1, 5] {
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
    std::fs::write(root.path().join("a.txt"), "a".repeat(8000)).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = batch(
        &app,
        json!([job("a.txt"), job("a.txt"), job("a.txt"), job("a.txt")]),
    );
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(
        result["data"]["results"][0]["fullResultInBatchDetail"],
        true
    );
    assert!(result["detail"]["ref"].is_string());
}
