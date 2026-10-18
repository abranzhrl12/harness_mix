use agent_harness::adapters;
use harness_domain::contracts::Request;
use serde_json::{Value, json};
#[path = "support/codegraph.rs"]
mod graph_fixture;

#[test]
fn llm_session_name_and_copyable_example_work_without_translation() {
    let root = tempfile::tempdir().unwrap();
    drop(graph_fixture::fixture(root.path()));
    let app = adapters::application(root.path()).unwrap();
    let boot = app.execute(Request::Bootstrap(
        serde_json::from_value(json!({})).unwrap(),
    ));
    let session = boot["data"]["session"].as_str().unwrap();
    assert_eq!(boot["data"]["sessionId"], session);
    let mut example = boot["data"]["exampleCall"]["arguments"].clone();
    assert_eq!(example["session"], session);
    assert!(example.get("sessionId").is_none());
    example["query"] = json!("target");
    let args: harness_domain::contracts::ExploreArgs =
        serde_json::from_value(example.clone()).unwrap();
    let response = app.execute(Request::Explore(args));
    assert_eq!(response["state"], "ok", "{response}");
    assert_eq!(response["data"]["focus"]["status"], "resolved");

    // Accept the exact mistake observed in Antigravity, without silently changing a session.
    let value = example.as_object_mut().unwrap().remove("session").unwrap();
    example["sessionId"] = value;
    let legacy: harness_domain::contracts::ExploreArgs =
        serde_json::from_value(example.clone()).unwrap();
    assert_eq!(legacy.session.as_deref(), Some(session));
    assert_eq!(app.execute(Request::Explore(legacy))["state"], "ok");
    example["session"] = json!("conflicting-session");
    assert!(serde_json::from_value::<harness_domain::contracts::ExploreArgs>(example).is_err());
    let reused = app.execute(Request::Bootstrap(
        serde_json::from_value(json!({"sessionId":session})).unwrap(),
    ));
    assert_eq!(reused["data"]["session"], session);
    assert_eq!(reused["data"]["rulesUnchanged"], true);
}

#[test]
fn investigation_profile_filters_bootstrap_and_blocks_operations_before_execution() {
    use harness_domain::tool_contract::ToolProfile;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("example.txt"), "old").unwrap();
    let app = adapters::application(root.path())
        .unwrap()
        .with_tool_profile(ToolProfile::Investigate);
    let boot = app.execute(Request::Bootstrap(
        serde_json::from_value(json!({})).unwrap(),
    ));
    assert_eq!(boot["data"]["toolProfile"], "investigate");
    assert_eq!(boot["data"]["supportedTools"].as_array().unwrap().len(), 13);
    assert_eq!(
        boot["data"]["supportedTools"],
        boot["data"]["toolContract"]["tools"]
    );
    assert!(boot["data"]["workflow"].get("fix").is_none());
    for name in boot["data"]["toolContract"]["features"]["batchTools"]
        .as_array()
        .unwrap()
    {
        assert!(ToolProfile::Investigate.allows(name.as_str().unwrap()));
    }
    let edit = app.execute(Request::Replace(
        serde_json::from_value(json!({
            "path":"example.txt","hash":harness_domain::hash(b"old"),"oldText":"old","newText":"new"
        }))
        .unwrap(),
    ));
    assert_eq!(edit["diagnostics"][0]["code"], "TOOL_PROFILE");
    assert_eq!(
        std::fs::read_to_string(root.path().join("example.txt")).unwrap(),
        "old"
    );
    let sync = app.execute(Request::Explore(
        serde_json::from_value(json!({"query":"main","syncIndex":true})).unwrap(),
    ));
    assert_eq!(sync["diagnostics"][0]["code"], "TOOL_PROFILE");
    assert!(!root.path().join(".codegraph").exists());
    let batch = app.execute(Request::Batch(
        serde_json::from_value(json!({"jobs":[
            {"tool":"harness_read","arguments":{"path":"example.txt","start":1,"end":1}},
            {"tool":"harness_sql","arguments":{"action":"status"}}
        ]}))
        .unwrap(),
    ));
    assert_eq!(batch["diagnostics"][0]["code"], "TOOL_PROFILE");
    let logs = app.execute(Request::Logs(
        serde_json::from_value(json!({"tool":"harness_read","limit":10})).unwrap(),
    ));
    assert!(
        logs["data"]["events"].as_array().unwrap().is_empty(),
        "no child should start"
    );
}

#[test]
fn edit_profile_adds_validated_edits_without_enabling_sql() {
    use harness_domain::tool_contract::ToolProfile;
    assert!(ToolProfile::parse("typo").is_err());
    assert_eq!(ToolProfile::Full.tools().len(), 28);
    assert_eq!(ToolProfile::Edit.tools().len(), 17);
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("example.txt"), "old").unwrap();
    let app = adapters::application(root.path())
        .unwrap()
        .with_tool_profile(ToolProfile::Edit);
    let edit = app.execute(Request::Replace(
        serde_json::from_value(json!({
            "path":"example.txt","hash":harness_domain::hash(b"old"),"oldText":"old","newText":"new"
        }))
        .unwrap(),
    ));
    assert_eq!(edit["state"], "ok", "{edit}");
    assert_eq!(
        std::fs::read_to_string(root.path().join("example.txt")).unwrap(),
        "new"
    );
    let sql = app.execute(Request::SqlStatus(
        serde_json::from_value(json!({})).unwrap(),
    ));
    assert_eq!(sql["diagnostics"][0]["code"], "TOOL_PROFILE");
    assert!(ToolProfile::Edit.allows("harness_verify"));
    assert!(ToolProfile::Edit.allows("harness_diagnostics"));
}

fn query(app: &adapters::NativeApplication, args: Value) -> Value {
    app.execute(Request::DataQuery(serde_json::from_value(args).unwrap()))
}

#[test]
fn llm_query_unicode_detail_and_result_reference() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let response = query(
        &app,
        json!({"data":"😀".repeat(300),"query":"@","maxChars":500}),
    );
    assert_eq!(response["state"], "ok", "{response}");
    assert_eq!(response["omitted"]["truncated"], true);
    assert!(
        response["data"]["preview"]
            .as_str()
            .unwrap()
            .encode_utf16()
            .count()
            <= 500
    );
    let reference = response["detail"]["ref"].as_str().unwrap();
    let projected = query(
        &app,
        json!({"resultRef":reference,"query":"length(result)"}),
    );
    assert_eq!(projected["state"], "ok", "{projected}");
    assert_eq!(projected["data"]["result"], 300);
}

#[test]
fn llm_query_redacts_before_projection_and_never_echoes_expression() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    std::fs::write(root.path().join("sample.json"), r#"{"password":"fixture-only-secret","nested":{"api_key":"fixture-only-key"},"message":"Password=fixture-only-text;","public":42}"#).unwrap();
    let response = query(&app, json!({"path":"sample.json","query":"@"}));
    assert_eq!(response["state"], "ok", "{response}");
    assert!(!response.to_string().contains("fixture-only"));
    assert_eq!(response["data"]["result"]["public"], 42);
    let response = query(
        &app,
        json!({"data":{},"query":"`{\"password\":\"literal-fixture-secret\"}`"}),
    );
    assert!(!response.to_string().contains("literal-fixture-secret"));
}

#[test]
fn llm_query_conflicting_sources_and_expired_refs_are_recoverable() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    for args in [
        json!({"data":{},"path":"sample.json","query":"@"}),
        json!({"data":{},"resultRef":"unknown","query":"@"}),
    ] {
        let response = query(&app, args);
        assert_eq!(response["state"], "error");
        assert_eq!(response["diagnostics"][0]["retryable"], false);
        assert_eq!(
            response["diagnostics"][0]["requiredAction"],
            "correct_arguments"
        );
    }
    let response = query(&app, json!({"resultRef":"unknown","query":"@"}));
    assert_eq!(response["diagnostics"][0]["code"], "DETAIL");
    assert_eq!(response["diagnostics"][0]["retryable"], false);
}

#[test]
fn llm_explore_exact_selectors_precede_homonym_limits() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    for i in 0..120 {
        db.execute(
            "INSERT INTO nodes VALUES(?1,'target','function','lib.rs',1)",
            [format!("aaa{i:03}")],
        )
        .unwrap();
    }
    std::fs::write(root.path().join("zz.rs"), "pub fn target() {}\n").unwrap();
    db.execute(
        "INSERT INTO nodes VALUES('zz','target','function','zz.rs',1)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO files VALUES('zz.rs',?1,NULL)",
        [harness_domain::hash(b"pub fn target() {}\n")],
    )
    .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let call =
        |options: Value| app.execute(Request::Explore(serde_json::from_value(options).unwrap()));
    for (options, id) in [
        (json!({"query":"target","symbolId":"t"}), "t"),
        (json!({"query":"target","filePath":"zz.rs"}), "zz"),
    ] {
        let result = call(options);
        assert_eq!(result["state"], "ok", "{result}");
        assert_eq!(result["data"]["candidates"][0]["id"], id);
        assert_eq!(result["data"]["candidates"].as_array().unwrap().len(), 1);
        assert_eq!(
            result["data"]["graph"]["truncation"]["targetSelection"],
            false
        );
    }
    db.execute_batch("ALTER TABLE nodes ADD COLUMN qualified_name TEXT; UPDATE nodes SET qualified_name='Module::target' WHERE id='t'").unwrap();
    let result = call(json!({"query":"target","container":"Module"}));
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["candidates"][0]["id"], "t");
    let broad = call(json!({"query":"target"}));
    assert_eq!(
        broad["data"]["graph"]["truncation"]["targetSelection"],
        true
    );
    assert_eq!(broad["data"]["candidates"].as_array().unwrap().len(), 20);
    let missing = call(json!({"query":"target","symbolId":"not-indexed"}));
    assert_eq!(missing["state"], "error");
    assert_eq!(missing["diagnostics"][0]["code"], "TARGET_NOT_RESOLVED");
    assert_eq!(missing["diagnostics"][0]["retryable"], false);
}

#[test]
fn llm_explore_reports_edge_and_depth_limits_separately() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    for i in 4..30 {
        db.execute("INSERT INTO edges VALUES(?1,'c','t','calls')", [i])
            .unwrap();
    }
    let app = adapters::application(root.path()).unwrap();
    let result = app.execute(Request::Explore(
        serde_json::from_value(json!({"query":"target"})).unwrap(),
    ));
    let flags = &result["data"]["graph"]["truncation"];
    assert_eq!(flags["targetSelection"], false);
    assert_eq!(flags["perNodeEdges"], true);
    assert_eq!(flags["depthBoundary"], true);
    assert_eq!(flags["totalEdges"], false);
}

#[test]
fn llm_bootstrap_reports_actual_registry_and_sql_batch_support() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = app.execute(Request::Bootstrap(
        serde_json::from_value(json!({})).unwrap(),
    ));
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["backend"]["name"], "rust");
    assert_eq!(
        result["data"]["backend"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(result["data"]["toolContract"]["contractVersion"], "2.3.0");
    assert_eq!(
        result["data"]["supportedTools"].as_array().unwrap().len(),
        28
    );
    for name in result["data"]["toolContract"]["features"]["batchTools"]
        .as_array()
        .unwrap()
    {
        serde_json::from_value::<harness_domain::contracts::BatchTool>(name.clone()).unwrap();
    }
    assert!(
        result["data"]["toolContract"]["features"]["batchTools"]
            .as_array()
            .unwrap()
            .contains(&json!("harness_sql"))
    );
}
