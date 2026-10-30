use serde_json::{Value, json};
#[path = "support/codegraph.rs"]
mod graph_fixture;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

struct Server {
    child: Child,
    messages: mpsc::Receiver<String>,
}

#[test]
fn structured_presentation_over_stdio_preserves_results_and_errors() {
    let root = tempfile::tempdir().unwrap();
    let source = "pub fn unique_marker() -> i32 { 42 }\n";
    std::fs::write(root.path().join("sample.rs"), source).unwrap();
    let mut server = Server::start_with_mode(root.path(), false, false, None, "full", true);
    server.initialize();
    let calls = [
        ("harness_bootstrap", json!({"compact":true})),
        ("harness_read", json!({"path":"sample.rs","start":1,"end":1})),
        ("harness_symbols", json!({"action":"read","path":"sample.rs","symbol":"unique_marker"})),
        ("harness_search", json!({"kind":"text","query":"unique_marker","scope":".","contextLines":1})),
        ("harness_batch", json!({"jobs":[
            {"tool":"harness_read","arguments":{"path":"sample.rs","start":1,"end":1}},
            {"tool":"harness_read","arguments":{"path":"missing.rs","start":1,"end":1}}
        ]})),
    ];
    for (i, (tool, arguments)) in calls.into_iter().enumerate() {
        let response = server.request(i as u64 + 2, "tools/call", json!({"name":tool,"arguments":arguments}));
        let result = &response["result"];
        let text = result["content"][0]["text"].as_str().unwrap();
        let envelope = &result["structuredContent"];
        assert_eq!(envelope["state"], if tool == "harness_batch" { "error" } else { "ok" }, "{response}");
        assert!(text.contains(tool), "{response}");
        assert!(!text.contains("pub fn unique_marker"), "{response}");
        match tool {
            "harness_bootstrap" => assert!(text.contains(envelope["data"]["session"].as_str().unwrap())),
            "harness_read" => assert_eq!(envelope["data"]["text"], source.trim_end()),
            "harness_symbols" => assert_eq!(envelope["data"]["text"], source.trim_end()),
            "harness_search" => assert!(envelope["data"]["matches"][0]["context"]["text"].as_str().unwrap().contains(source.trim_end())),
            "harness_batch" => {
                assert_eq!(envelope["data"]["results"][0]["data"]["text"], source.trim_end());
                assert_eq!(envelope["data"]["results"][1]["state"], "error");
                let diagnostic = &envelope["data"]["results"][1]["diagnostics"][0];
                assert!(text.contains(diagnostic["code"].as_str().unwrap()), "{response}");
            }
            _ => unreachable!(),
        }
    }
    let mut legacy = Server::start_with_mode(root.path(), false, false, Some("legacy"), "full", true);
    legacy.initialize();
    let response = legacy.request(2, "tools/call", json!({"name":"harness_read","arguments":{"path":"sample.rs","start":1,"end":1}}));
    assert!(response["result"]["content"][0]["text"].as_str().unwrap().contains(source.trim_end()), "{response}");
    assert_eq!(response["result"]["structuredContent"]["data"]["text"], source.trim_end());
}

#[test]
fn explore_stdio_default_avoids_duplicate_text_and_unrelated_lineage() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    drop(db);
    std::fs::write(root.path().join("app.js"), "function target() { return target; }").unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    for (id, view) in [(2, "summary"), (3, "full")] {
        let response = server.request(id, "tools/call", json!({"name":"harness_explore","arguments":{
            "filePath":"lib.rs","symbol":"target","view":view,"syncIndex":false
        }}));
        let result = &response["result"];
        let data = &result["structuredContent"]["data"];
        let text = result["content"][0]["text"].as_str().unwrap();
        assert_eq!(result["structuredContent"]["state"], "ok", "{response}");
        assert!(text.contains("harness_explore"));
        assert!(!text.contains("pub fn target"));
        assert!(data.get("dataTrace").is_none() && data.get("crossLayerTrace").is_none(), "{response}");
        assert!(data["fileFreshness"].is_object());
        assert!(data["definitions"].is_array());
        assert!(result["structuredContent"]["detail"]["ref"].is_string());
    }
}

fn direct_tool_payload(response: &Value) -> Value {
    // Successful source payloads now render a lean markdown text channel; the
    // canonical envelope lives in structuredContent. Flatten it the same way
    // the legacy direct text channel did (data fields plus envelope metadata).
    if let Some(envelope) = response["result"].get("structuredContent") {
        let mut payload = envelope["data"].clone();
        if let Some(map) = payload.as_object_mut() {
            let ok = envelope["state"] == json!("ok");
            for (key, value) in envelope.as_object().cloned().unwrap_or_default() {
                if key == "data" || key == "scope" || (ok && key == "state") {
                    continue;
                }
                map.insert(key, value);
            }
        }
        return payload;
    }
    assert_eq!(response["result"]["content"].as_array().unwrap().len(), 1);
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn ergonomic_search_and_bootstrap_errors_work_over_mcp() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.rs"), "needle\n").unwrap();
    std::fs::write(root.path().join("b.txt"), "needle\n").unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let response = server.request(
        2,
        "tools/call",
        json!({
            "name":"harness_search", "arguments":{
                "query":"needle", "scope":".", "path":".", "globs":["*.rs"]
            }
        }),
    );
    let envelope = &response["result"]["structuredContent"];
    assert_eq!(envelope["state"], "ok", "{response}");
    let matches = envelope["data"]["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1, "{response}");
    assert_eq!(matches[0]["file"], "a.rs", "{response}");
    let response = server.request(
        3,
        "tools/call",
        json!({
            "name":"harness_bootstrap", "arguments":{"action":"env"}
        }),
    );
    let message = response.to_string();
    assert!(
        message.contains("action") && message.contains("harness_doctor"),
        "{message}"
    );
}

#[test]
fn sql_defaults_describe_aliases_and_replace_base_hash_work_over_mcp() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "before\n").unwrap();
    std::fs::create_dir(root.path().join(".agents")).unwrap();
    std::fs::write(
        root.path().join(".agents/harness.config.json"),
        r#"{"sqlServer":{"enabled":false},"postgresql":{"enabled":false}}"#,
    )
    .unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let catalog = server.request(2, "tools/list", json!({}));
    let tools = catalog["result"]["tools"].as_array().unwrap();
    let search_tool = tools
        .iter()
        .find(|tool| tool["name"] == "harness_search")
        .unwrap();
    let search_description = search_tool["description"].as_str().unwrap();
    for field in [
        "matches[].file",
        "matches[].context.text",
        "matches[].hash",
        "context:{start,end,text}",
    ] {
        assert!(
            search_description.contains(field),
            "Search fast-track missing {field}: {search_tool}"
        );
    }
    let replace = tools
        .iter()
        .find(|tool| tool["name"] == "harness_replace")
        .unwrap();
    let replace_description = replace["description"].as_str().unwrap();
    assert!(
        replace_description.contains("matches[].context.text"),
        "{replace}"
    );
    assert!(
        replace_description.contains("hash es opcional"),
        "{replace}"
    );
    assert!(
        replace["inputSchema"]["properties"]
            .get("baseHash")
            .is_some(),
        "{replace}"
    );
    let sql_tool = tools
        .iter()
        .find(|tool| tool["name"] == "harness_sql")
        .unwrap();
    let schema_text = sql_tool["inputSchema"].to_string();
    assert!(
        schema_text.contains("objectType")
            && schema_text.contains("routine")
            && schema_text.contains("procedure"),
        "{sql_tool}"
    );
    let read = server.request(
        3,
        "tools/call",
        json!({"name":"harness_read","arguments":{"path":"a.txt","start":1,"lines":1}}),
    );
    assert!(read["result"]["structuredContent"]["data"]["hash"].is_string());
    let replaced = server.request(4, "tools/call", json!({"name":"harness_replace","arguments":{"path":"a.txt","oldText":"before","newText":"after"}}));
    assert_eq!(
        replaced["result"]["structuredContent"]["state"], "ok",
        "{replaced}"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
        "after\n"
    );
    // These calls must reach the adapter, without attempting a real configured DB.
    for (id, arguments) in [
        (5, json!({"sql":"SELECT 1"})),
        (
            6,
            json!({"action":"describe","engine":"sqlserver","name":"dbo.Example","objectType":"procedure"}),
        ),
    ] {
        let result = server.request(
            id,
            "tools/call",
            json!({"name":"harness_sql","arguments":arguments}),
        );
        assert!(
            result["result"]["structuredContent"]["state"].is_string(),
            "SQL input did not reach adapter: {result}"
        );
        assert!(result.get("error").is_none(), "{result}");
    }
    let invalid = server.request(7, "tools/call", json!({"name":"harness_sql","arguments":{"action":"query","engine":42,"credentials":"DO_NOT_ECHO"}}));
    let message = invalid.to_string();
    assert!(
        message.contains("Unknown field `credentials`")
            && message.contains("REQUIRED: {sql: string}")
            && message.contains("parameters")
            && message.contains("engine"),
        "{message}"
    );
    assert!(!message.contains("DO_NOT_ECHO"), "{message}");
}

#[test]
fn replace_recovery_diagnostics_survive_mcp_without_publishing() {
    let root = tempfile::tempdir().unwrap();
    let original = "first\r\nsecond line\r\n";
    std::fs::write(root.path().join("recover.txt"), original).unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    for (id, preview, hash, old_text) in [
        (2, false, harness_domain::hash(b"old version"), "first"),
        (3, true, harness_domain::hash(b"old version"), "first"),
        (
            4,
            false,
            harness_domain::hash(original.as_bytes()),
            // Not present in the file: match failure with a healthy hash is the
            // recovery diagnostic path. Exact-looking but whitespace-shifted
            // needles would match, because the matcher tolerates spacing.
            "absent third line",
        ),
        (
            5,
            true,
            harness_domain::hash(original.as_bytes()),
            "absent third line",
        ),
    ] {
        let response = server.request(
            id,
            "tools/call",
            json!({
                "name":"harness_replace","arguments":{
                    "path":"recover.txt","hash":hash,"oldText":old_text,
                    "newText":"changed","preview":preview
                }
            }),
        );
        assert_eq!(response["result"]["isError"], true, "{response}");
        let envelope = &response["result"]["structuredContent"];
        assert_eq!(envelope["state"], "error", "{response}");
        assert_eq!(envelope["data"]["changed"], false, "{response}");
        assert_eq!(
            envelope["data"]["currentHash"],
            harness_domain::hash(original.as_bytes()),
            "{response}"
        );
        assert!(
            !envelope["diagnostics"].as_array().unwrap().is_empty(),
            "{response}"
        );
        assert_eq!(
            std::fs::read(root.path().join("recover.txt")).unwrap(),
            original.as_bytes()
        );
    }
}

#[test]
fn compact_method_outline_is_advertised_and_works_over_mcp() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("Controller.cs"),
        "class Controller { public Controller() {} private int field; [HttpPost] public string Save(int id) { return \"ok\"; } public string Save(string id) { return id; } }").unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let catalog = server.request(2, "tools/list", json!({}));
    let symbols = catalog["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "harness_symbols")
        .unwrap();
    assert!(
        symbols["inputSchema"].to_string().contains("filterKind"),
        "{symbols}"
    );
    assert!(
        symbols["inputSchema"].to_string().contains("compact"),
        "{symbols}"
    );
    let response = server.request(
        3,
        "tools/call",
        json!({
            "name":"harness_symbols","arguments":{
                "action":"outline","path":"Controller.cs","filterKind":"method","view":"compact"
            }
        }),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    let items = response["result"]["structuredContent"]["data"]["items"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 2, "{response}");
    for item in items {
        assert_eq!(item["name"], "Save", "{response}");
        assert!(item["symbolId"].is_string(), "{response}");
        assert!(item.get("doc").is_none(), "{response}");
        assert!(item.get("text").is_none(), "{response}");
    }
    assert!(
        items
            .iter()
            .any(|item| item.to_string().contains("HttpPost")),
        "{response}"
    );
}

#[test]
fn compatibility_aliases_work_through_mcp_and_are_advertised() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.cs"), "class Controller {}\n").unwrap();
    std::fs::write(root.path().join("b.js"), "const Controller = 1;\n").unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let catalog = server.request(2, "tools/list", json!({}));
    let tools = catalog["result"]["tools"].as_array().unwrap();
    let search = tools
        .iter()
        .find(|t| t["name"] == "harness_search")
        .unwrap();
    assert!(
        search["inputSchema"]["properties"].get("path").is_some(),
        "{search}"
    );
    assert!(
        search["inputSchema"]["properties"]
            .get("includes")
            .is_some()
    );
    let read = tools.iter().find(|t| t["name"] == "harness_read").unwrap();
    assert!(
        read["inputSchema"]["properties"].get("file").is_some(),
        "{read}"
    );
    let result = server.request(3, "tools/call", json!({"name":"harness_search","arguments":{"kind":"text","query":"Controller","path":".","includes":["*.cs"]}}));
    let data = &result["result"]["structuredContent"];
    assert_eq!(data["state"], "ok", "{result}");
    assert_eq!(
        data["data"]["matches"].as_array().unwrap().len(),
        1,
        "{result}"
    );
    let result = server.request(
        4,
        "tools/call",
        json!({"name":"harness_read","arguments":{"file":"a.cs","start":1,"lines":1}}),
    );
    assert_eq!(
        result["result"]["structuredContent"]["state"], "ok",
        "{result}"
    );
    let boot = server.request(
        5,
        "tools/call",
        json!({"name":"harness_bootstrap","arguments":{"compact":true}}),
    );
    let session = boot["result"]["structuredContent"]["data"]["session"]
        .as_str()
        .unwrap();
    let result = server.request(6, "tools/call", json!({"name":"harness_session","arguments":{"action":"checkpoint","session":session,"data":{"goal":"compatibility","decisions":["accept aliases"]}}}));
    assert_eq!(
        result["result"]["structuredContent"]["state"], "ok",
        "{result}"
    );
}

#[test]
#[ignore = "opt-in read-only timings on an authorized existing project; no index writes"]
fn real_project_command_timings() {
    let root = std::path::PathBuf::from(
        std::env::var_os("HARNESS_BENCH_ROOT").expect("HARNESS_BENCH_ROOT"),
    );
    for round in 0..3 {
        let started = std::time::Instant::now();
        let mut server = Server::start_with_runtime(&root, false, true);
        server.initialize();
        println!(
            "{}",
            json!({"round":round,"operation":"startup_and_mcp_initialize","milliseconds":started.elapsed().as_secs_f64()*1000.0})
        );
        let started = std::time::Instant::now();
        let boot = server.request(
            2,
            "tools/call",
            json!({"name":"harness_bootstrap","arguments":{}}),
        );
        let boot = &boot["result"]["structuredContent"];
        assert_eq!(boot["state"], "ok", "{boot}");
        let session = boot["data"]["session"].as_str().unwrap();
        println!(
            "{}",
            json!({"round":round,"operation":"bootstrap_initial","milliseconds":started.elapsed().as_secs_f64()*1000.0,"responseBytes":serde_json::to_vec(boot).unwrap().len()})
        );
        for repetition in 0..5 {
            for (tool, arguments) in [
                ("harness_bootstrap", json!({"session":session})),
                (
                    "harness_explore",
                    json!({"session":session,"query":"main","filePath":"crates/api/src/main.rs","syncIndex":false}),
                ),
                (
                    "harness_symbols",
                    json!({"action":"read","session":session,"path":"crates/api/src/main.rs","symbol":"main"}),
                ),
                (
                    "harness_search",
                    json!({"session":session,"kind":"files","query":"main.rs","scope":"crates/api/src","limit":4}),
                ),
            ] {
                let started = std::time::Instant::now();
                let response =
                    server.request(3, "tools/call", json!({"name":tool,"arguments":arguments}));
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                let payload = if response["result"].get("structuredContent").is_some() {
                    response["result"]["structuredContent"].clone()
                } else { direct_tool_payload(&response) };
                let envelope = &payload;
                assert_ne!(response["result"]["isError"], true, "{tool}: {envelope}");
                println!(
                    "{}",
                    json!({"round":round,"repetition":repetition,"operation":tool,"milliseconds":elapsed,"responseBytes":serde_json::to_vec(envelope).unwrap().len(),"timings":envelope["data"]["timings"]})
                );
            }
        }
    }
}

#[test]
#[ignore = "opt-in response/log audit on an authorized project; read-only"]
fn real_project_payload_and_log_audit() {
    let root = std::path::PathBuf::from(
        std::env::var_os("HARNESS_BENCH_ROOT").expect("HARNESS_BENCH_ROOT"),
    );
    let mut server = Server::start_with_runtime(&root, false, true);
    server.initialize();
    let boot = server.request(
        2,
        "tools/call",
        json!({"name":"harness_bootstrap","arguments":{}}),
    );
    let session = boot["result"]["structuredContent"]["data"]["session"]
        .as_str()
        .unwrap();
    let mut previous_graph = None;
    for budget in [6000, 12000] {
        let result = server.request(3, "tools/call", json!({"name":"harness_explore","arguments":{"query":"main","filePath":"crates/api/src/main.rs","maxFiles":2,"maxChars":budget,"syncIndex":false,"session":session}}));
        let envelope = direct_tool_payload(&result);
        assert_ne!(result["result"]["isError"], true, "{envelope}");
        let request_id = envelope["requestId"].as_str().unwrap();
        let logs = server.request(4, "tools/call", json!({"name":"harness_doctor","arguments":{"action":"logs","requestId":request_id,"limit":10,"session":session}}));
        assert_eq!(logs["result"]["structuredContent"]["state"], "ok");
        let events = logs["result"]["structuredContent"]["data"]["events"]
            .as_array()
            .unwrap();
        let completed = events
            .iter()
            .find(|e| e["requestId"] == request_id && e["kind"] == "completed")
            .expect("correlated terminal log");
        assert_eq!(completed["state"], "ok", "{completed}");
        let data = &envelope;
        let graph = json!({"nodes":data["graph"]["nodes"],"edges":data["graph"]["edges"]});
        if let Some(previous) = &previous_graph {
            assert_eq!(previous, &graph);
        }
        previous_graph = Some(graph);
        let definitions = data["definitions"].as_array().unwrap();
        println!(
            "{}",
            json!({
                "maxChars":budget,"nativeLogMicros":completed["durationMicros"],"logState":completed["state"],
                "wireBytes":serde_json::to_vec(&result).unwrap().len(),
                "modelJsonBytes":serde_json::to_vec(&envelope).unwrap().len(),
                "textChannelBytes":serde_json::to_vec(&result["result"]["content"]).unwrap().len(),
                "markdownBytes":data["text"].as_str().unwrap().len(),
                "graphBytes":serde_json::to_vec(&data["graph"]).unwrap().len(),
                "definitionMetadataBytes":serde_json::to_vec(&data["definitions"]).unwrap().len(),
                "completeBodies":definitions.iter().filter(|d| d["sourceStatus"]=="complete").count(),
                "partialBodies":definitions.iter().filter(|d| d["sourceStatus"]=="partial").count(),
                "previewTruncated":data["contextTruncation"]["textPreview"],
                "nodeCount":data["graph"]["nodes"].as_array().unwrap().len(),
                "edgeCount":data["graph"]["edges"].as_array().unwrap().len()
            })
        );
    }
}

#[test]
#[ignore = "opt-in comparison of client presentations; existing project, read-only"]
fn real_project_context_presentation() {
    let root = std::path::PathBuf::from(
        std::env::var_os("HARNESS_BENCH_ROOT").expect("HARNESS_BENCH_ROOT"),
    );
    let mut original_graph = None;
    for context in [false, true] {
        let mut server = Server::start_with_presentation(&root, false, true, context);
        server.initialize();
        let boot = server.request(
            2,
            "tools/call",
            json!({"name":"harness_bootstrap","arguments":{}}),
        );
        let session = boot["result"]["structuredContent"]["data"]["session"]
            .as_str()
            .unwrap();
        let response = server.request(3, "tools/call", json!({"name":"harness_explore","arguments":{"query":"main","filePath":"crates/api/src/main.rs","maxFiles":2,"maxChars":12000,"syncIndex":false,"session":session}}));
        let envelope = direct_tool_payload(&response);
        assert_ne!(response["result"]["isError"], true, "{envelope}");
        let data = &envelope;
        let graph = json!({"nodes":data["graph"]["nodes"],"edges":data["graph"]["edges"]});
        if let Some(original) = &original_graph {
            assert_eq!(original, &graph);
        }
        original_graph = Some(graph);
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        // The full view renders the lean reading channel; the structured
        // envelope is canonical and identical for both presentation flags.
        assert!(text.starts_with("## Graph context"), "{text}");
        let envelope_obj = &response["result"]["structuredContent"];
        assert_eq!(envelope["graph"], envelope_obj["data"]["graph"]);
        assert!(text.contains("not_certified"));
        assert!(text.contains("sourceStatus"));
        println!(
            "{}",
            json!({"presentation":if context {"context"} else {"legacy"},
            "wireBytes":serde_json::to_vec(&response).unwrap().len(),
            "textChannelBytes":text.len(),"modelJsonBytes":serde_json::to_vec(&envelope).unwrap().len(),
            "nodeCount":data["graph"]["nodes"].as_array().unwrap().len(),
            "edgeCount":data["graph"]["edges"].as_array().unwrap().len(),
            "completeBodies":data["definitions"].as_array().unwrap().iter().filter(|d| d["sourceStatus"]=="complete").count()})
        );
    }
}

#[test]
fn tool_profiles_filter_registry_and_enforce_direct_and_batch_calls() {
    for (profile, count) in [("investigate", 12), ("edit", 18)] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("example.txt"), "old").unwrap();
        let mut server =
            Server::start_with_profile(root.path(), false, false, false, profile, true);
        server.initialize();
        let catalog = server.request(2, "tools/list", json!({}));
        let tools = catalog["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), count);
        assert_eq!(
            tools
                .iter()
                .filter(|t| t["_meta"]["antigravity/alwaysLoad"] == true)
                .count(),
            2
        );
        assert!(
            !tools
                .iter()
                .any(|t| t["name"].as_str().unwrap().starts_with("harness_sql"))
        );
        let batch_schema =
            tools.iter().find(|t| t["name"] == "harness_batch").unwrap()["inputSchema"].to_string();
        assert!(!batch_schema.contains("harness_sql"));
        assert!(batch_schema.contains("harness_read"));
        assert!(batch_schema.contains("harness_explore"));
        assert_eq!(
            tools.iter().any(|t| t["name"] == "harness_replace"),
            profile == "edit"
        );
        let boot = server.request(
            3,
            "tools/call",
            json!({"name":"harness_bootstrap","arguments":{}}),
        );
        let data = &boot["result"]["structuredContent"]["data"];
        assert!(data.get("supportedTools").is_none());
        assert!(data.get("quickGuide").is_none());
        assert!(data.get("logging").is_none());
        assert!(data.get("executionPolicy").is_none());
        let session_value = data["session"].as_str().unwrap().to_owned();
        assert_eq!(data["sessionId"], session_value);
        let blocked = server.request(
            4,
            "tools/call",
            json!({"name":"harness_sql","arguments":{"action":"status"}}),
        );
        assert!(
            blocked.get("error").is_some() || blocked["result"]["isError"] == true,
            "{blocked}"
        );
        let sync = server.request(
            5,
            "tools/call",
            json!({"name":"harness_explore","arguments":{"query":"main","syncIndex":true}}),
        );
        assert_eq!(
            direct_tool_payload(&sync)["diagnostics"][0]["code"],
            "TOOL_PROFILE"
        );
        let batch = server.request(
            6,
            "tools/call",
            json!({"name":"harness_batch","arguments":{"jobs":[
                {"tool":"harness_read","arguments":{"path":"example.txt","start":1,"end":1}},
                {"tool":"harness_sql","arguments":{"action":"status"}}
            ]}}),
        );
        assert_eq!(
            batch["result"]["structuredContent"]["diagnostics"][0]["code"],
            "TOOL_PROFILE"
        );
        let logs = server.request(
            7,
            "tools/call",
            json!({"name":"harness_doctor","arguments":{"action":"logs","tool":"harness_read","limit":10}}),
        );
        let events = logs["result"]["structuredContent"]["data"]["events"].as_array();
        if let Some(events) = events {
            assert!(events.is_empty());
        } else {
            assert_eq!(
                logs["result"]["structuredContent"]["diagnostics"][0]["code"],
                "TOOL_PROFILE"
            );
        }
        assert_eq!(
            std::fs::read_to_string(root.path().join("example.txt")).unwrap(),
            "old"
        );
        assert!(!root.path().join(".codegraph").exists());
    }
}

#[test]
fn eager_tools_hint_can_be_disabled_without_changing_full_registry() {
    let root = tempfile::tempdir().unwrap();
    let mut server = Server::start_with_profile(root.path(), false, false, false, "full", false);
    server.initialize();
    let catalog = server.request(2, "tools/list", json!({}));
    let tools = catalog["result"]["tools"].as_array().unwrap();
    let mut actual: Vec<_> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    actual.sort();
    let mut expected = harness_domain::tool_contract::tool_names("rust");
    expected.sort();
    assert_eq!(actual, expected.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(
        tools
            .iter()
            .all(|t| t["_meta"]["antigravity/alwaysLoad"] != true)
    );
}

#[test]
fn tool_profiles_reject_unknown_startup_configuration() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agent-harness"))
        .arg("--root")
        .arg(root.path())
        .env("HARNESS_TOOL_PROFILE", "investigate-typo")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("HARNESS_TOOL_PROFILE must be"));
    assert!(!root.path().join(".codegraph").exists());
}

#[test]
fn automatic_index_writes_require_explicit_valid_configuration_and_full_profile() {
    let root = tempfile::tempdir().unwrap();
    for (setting, profile, expected) in [
        ("yes", "full", "HARNESS_INDEX_WATCH must be 0 or 1"),
        ("1", "investigate", "requires the full tool profile"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-harness"))
            .arg("--root")
            .arg(root.path())
            .env("HARNESS_TOOL_PROFILE", profile)
            .env("HARNESS_INDEX_WATCH", setting)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
        assert!(!root.path().join(".codegraph").exists());
    }
}

impl Server {
    fn start(root: &std::path::Path) -> Self {
        Self::start_with_logging(root, false)
    }
    fn start_with_logging(root: &std::path::Path, persist: bool) -> Self {
        Self::start_with_runtime(root, persist, false)
    }
    fn start_with_runtime(root: &std::path::Path, persist: bool, keep_path: bool) -> Self {
        Self::start_with_presentation(root, persist, keep_path, false)
    }
    fn start_with_presentation(
        root: &std::path::Path,
        persist: bool,
        keep_path: bool,
        context: bool,
    ) -> Self {
        Self::start_with_profile(root, persist, keep_path, context, "full", true)
    }
    fn start_with_profile(
        root: &std::path::Path,
        persist: bool,
        keep_path: bool,
        context: bool,
        profile: &str,
        eager: bool,
    ) -> Self {
        Self::start_with_mode(root, persist, keep_path, if context { Some("context") } else { None }, profile, eager)
    }
    fn start_with_mode(root: &std::path::Path, persist: bool, keep_path: bool, mode: Option<&str>, profile: &str, eager: bool) -> Self {
        let executable = std::env::var_os("HARNESS_NATIVE_BINARY")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_agent-harness").into());
        let mut command = Command::new(executable);
        if !keep_path {
            command.env("PATH", "");
        }
        command.arg("--root").arg(root);
        command
            .env("HARNESS_TOOL_PROFILE", profile)
            .env_remove("HARNESS_INDEX_WATCH")
            .env("HARNESS_MCP_EAGER_TOOLS", if eager { "1" } else { "0" });
        if let Some(mode) = mode {
            command.env("HARNESS_MCP_PRESENTATION", mode);
        } else {
            command.env_remove("HARNESS_MCP_PRESENTATION");
        }
        if persist {
            command.arg("--persist-logs");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if send.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Self { child, messages }
    }
    fn send(&mut self, value: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{value}").unwrap();
        stdin.flush().unwrap();
    }
    fn initialize(&mut self) {
        let init = self.request(1, "initialize", json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"compatibility-test","version":"1"}}));
        assert!(init.get("error").is_none(), "{init}");
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }
    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        loop {
            let line = self
                .messages
                .recv_timeout(Duration::from_secs(10))
                .expect("MCP response timeout");
            let response: Value =
                serde_json::from_str(&line).expect("stdout must contain only JSON-RPC");
            if response["id"] == id {
                return response;
            }
        }
    }
}

#[test]
fn graph_exploration_reads_existing_index_with_empty_path() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    db.execute("INSERT INTO edges VALUES(4,'d','c','calls')", [])
        .unwrap();
    drop(db);
    let mut server = Server::start(root.path());
    server.initialize();
    let result = server.request(
        2,
        "tools/call",
        json!({"name":"harness_explore","arguments":{"query":"target"}}),
    );
    let result = direct_tool_payload(&result);
    assert!(result.get("state").is_none(), "{result}");
    assert_eq!(
        result["graph"]["edges"].as_array().unwrap().len(),
        2
    );
    assert_eq!(result["focus"]["status"], "resolved");
    let summary = server.request(6, "tools/call", json!({"name":"harness_explore","arguments":{"filePath":"lib.rs","symbol":"target","view":"summary"}}));
    let summary = direct_tool_payload(&summary);
    assert!(summary.get("text").is_none());
    assert_eq!(summary["focus"]["status"], "resolved");
    assert!(summary["graph"]["relations"].as_array().is_some_and(|items| !items.is_empty()));
    let recovered = server.request(7, "tools/call", json!({"name":"harness_detail","arguments":{"ref":summary["detail"]["ref"],"maxChars":16000}}));
    let recovered: Value = serde_json::from_str(recovered["result"]["structuredContent"]["data"]["text"].as_str().unwrap()).unwrap();
    assert_eq!(recovered["graph"]["edges"], result["graph"]["edges"]);
    let deeper = server.request(3, "tools/call", json!({"name":"harness_explore","arguments":{"query":"target","direction":"outbound","maxDepth":2}}));
    let deeper = direct_tool_payload(&deeper);
    assert!(deeper.get("state").is_none(), "{deeper}");
    assert_eq!(
        deeper["graph"]["traversal"]["nodeDepths"]["c"]["outbound"],
        2
    );
    let matching = server.request(4, "tools/call", json!({"name":"harness_explore","arguments":{"query":"target","requireMatchingFiles":true}}));
    assert_eq!(
        direct_tool_payload(&matching)["fileFreshness"]["status"],
        "matches_observed_files"
    );
    std::fs::write(root.path().join("lib.rs"), "pub fn target() {}\n").unwrap();
    let stale = server.request(5, "tools/call", json!({"name":"harness_explore","arguments":{"query":"target","requireMatchingFiles":true}}));
    assert_eq!(
        direct_tool_payload(&stale)["diagnostics"][0]["code"],
        "INDEX_STALE"
    );
}

#[test]
fn syntax_search_round_trip_without_node() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("lib.rs"),
        "// fn fake() {}\npub fn actual() {}\n",
    )
    .unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let result = server.request(
        2,
        "tools/call",
        json!({"name":"harness_search","arguments":{
            "kind":"syntax","query":"function_item:actual","scope":"lib.rs"
        }}),
    );
    let result = &result["result"]["structuredContent"];
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["data"]["matches"][0]["name"], "actual");
    assert_eq!(result["data"]["syntaxOnly"], true);
    let rejected = server.request(
        3,
        "tools/call",
        json!({"name":"harness_search","arguments":{
            "kind":"syntax","query":"fn $NAME() {}","scope":"lib.rs"
        }}),
    );
    assert_eq!(
        rejected["result"]["structuredContent"]["diagnostics"][0]["code"],
        "ARGUMENT"
    );
}

#[test]
fn dotnet_verification_reports_missing_sdk_without_node() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("example.csproj"), "<Project/>").unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let result = server.request(
        2,
        "tools/call",
        json!({"name":"harness_verify","arguments":{
            "language":"csharp","scope":"example.csproj","action":"build"
        }}),
    );
    let result = &result["result"]["structuredContent"];
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(result["diagnostics"][0]["code"], "PROCESS_EXECUTABLE");
}

#[test]
fn javascript_verification_reports_missing_runtime_without_node() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("example.js"), "const x=1;\n").unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    // Fast per-file build validation runs in-process (no Node required); the Node
    // test runner is the path that needs the runtime executable.
    let result = server.request(
        2,
        "tools/call",
        json!({"name":"harness_verify","arguments":{
            "language":"javascript","scope":"example.js","action":"test"
        }}),
    );
    let result = &result["result"]["structuredContent"];
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(result["diagnostics"][0]["code"], "PROCESS_EXECUTABLE");
}

#[test]
fn typescript_and_tsx_builds_work_without_node_and_do_not_execute_code() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("example.ts"),
        "const value: number = 'not type checked'; throw new Error('not executed');",
    )
    .unwrap();
    std::fs::write(
        root.path().join("example.tsx"),
        "export const view = <span/>; throw new Error('not executed');",
    )
    .unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    for (id, path) in [(2, "example.ts"), (3, "example.tsx")] {
        let result = server.request(id, "tools/call", json!({"name":"harness_verify","arguments":{"language":"typescript","scope":path,"action":"build"}}));
        let envelope = &result["result"]["structuredContent"];
        assert_eq!(envelope["state"], "ok", "{envelope}");
        assert_eq!(
            envelope["data"]["verificationMode"],
            "typescript_syntax_only"
        );
        assert_eq!(envelope["data"]["typeCheckPerformed"], false);
        assert_eq!(envelope["data"]["codeExecuted"], false);
    }
}

#[test]
fn python_verification_reports_missing_interpreter_without_node() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("example.py"), "pass\n").unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let result = server.request(
        2,
        "tools/call",
        json!({"name":"harness_verify","arguments":{
            "language":"python","scope":"example.py","action":"build"
        }}),
    );
    let result = &result["result"]["structuredContent"];
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(result["diagnostics"][0]["code"], "PROCESS_EXECUTABLE");
}

#[test]
fn go_verification_reports_missing_toolchain_without_node() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("go.mod"),
        "module example.test/fixture\ngo 1.20\n",
    )
    .unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let result = server.request(
        2,
        "tools/call",
        json!({"name":"harness_verify","arguments":{
            "language":"go","scope":".","action":"build"
        }}),
    );
    let result = &result["result"]["structuredContent"];
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(result["diagnostics"][0]["code"], "PROCESS_EXECUTABLE");
}

#[cfg(windows)]
#[test]
fn xml_config_and_external_scope_round_trip_without_node() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("Web.config"),r#"<configuration><appSettings><add key="api" value="protocol-private"/></appSettings></configuration>"#).unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let boot = server.request(
        2,
        "tools/call",
        json!({"name":"harness_bootstrap","arguments":{}}),
    );
    let session = boot["result"]["structuredContent"]["data"]["sessionId"]
        .as_str()
        .unwrap();
    let path = outside.path().to_string_lossy();
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    let granted=server.request(3,"tools/call",json!({"name":"harness_external_scope","arguments":{"session":session,"action":"grant","root":path,"userAuthorized":true}}));
    assert_eq!(
        granted["result"]["structuredContent"]["state"], "ok",
        "{granted}"
    );
    let scope = granted["result"]["structuredContent"]["data"]["scopeId"]
        .as_str()
        .unwrap();
    let config=server.request(4,"tools/call",json!({"name":"harness_config","arguments":{"session":session,"externalScope":scope,"path":"Web.config","section":"appSettings"}}));
    assert_eq!(
        config["result"]["structuredContent"]["state"], "ok",
        "{config}"
    );
    assert!(!config.to_string().contains("protocol-private"));
    std::fs::write(
        outside.path().join("appsettings.json"),
        r#"{"Settings":{"protocol-key-private":"protocol-json-private"}}"#,
    )
    .unwrap();
    let config = server.request(7, "tools/call", json!({"name":"harness_config","arguments":{"session":session,"externalScope":scope,"path":"appsettings.json","section":"Settings"}}));
    assert_eq!(
        config["result"]["structuredContent"]["state"], "ok",
        "{config}"
    );
    assert!(!config.to_string().contains("protocol-key-private"));
    assert!(!config.to_string().contains("protocol-json-private"));
    let raw=server.request(5,"tools/call",json!({"name":"harness_read","arguments":{"session":session,"externalScope":scope,"path":"Web.config","start":1,"end":1}}));
    assert_eq!(
        raw["result"]["structuredContent"]["diagnostics"][0]["code"],
        "PATH"
    );
    let revoked=server.request(6,"tools/call",json!({"name":"harness_external_scope","arguments":{"session":session,"action":"revoke","scopeId":scope}}));
    assert_eq!(
        revoked["result"]["structuredContent"]["data"]["revoked"],
        true
    );
}

#[cfg(windows)]
#[test]
fn persistent_cli_logs_survive_server_restart_without_node() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("fixture.txt"), "persistent CLI").unwrap();
    let mut server = Server::start_with_logging(root.path(), true);
    server.initialize();
    let read = server.request(
        2,
        "tools/call",
        json!({"name":"harness_read","arguments":{"path":"fixture.txt","start":1,"end":1}}),
    );
    let envelope = &read["result"]["structuredContent"];
    assert_eq!(envelope["logPersistence"]["saved"], true, "{read}");
    let id = envelope["requestId"].as_str().unwrap().to_owned();
    drop(server);
    let mut restarted = Server::start_with_logging(root.path(), true);
    restarted.initialize();
    let logs = restarted.request(
        2,
        "tools/call",
        json!({"name":"harness_doctor","arguments":{"action":"logs","requestId":id}}),
    );
    assert_eq!(
        logs["result"]["structuredContent"]["data"]["events"][0]["historical"], true,
        "{logs}"
    );
}

#[test]
fn search_details_and_session_restore_work_without_node_or_ripgrep() {
    let root = tempfile::tempdir().unwrap();
    let rules = "😀".repeat(6001);
    std::fs::create_dir_all(root.path().join(".agents/rules")).unwrap();
    std::fs::write(
        root.path().join(".agents/rules/00-mandatory-tools.md"),
        &rules,
    )
    .unwrap();
    for name in ["a.txt", "b.txt", "secret.txt", "ignored.txt"] {
        std::fs::write(root.path().join(name), "needle 😀\n").unwrap();
    }
    std::fs::write(root.path().join(".gitignore"), "ignored.txt\n").unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let boot = server.request(
        2,
        "tools/call",
        json!({"name":"harness_bootstrap","arguments":{}}),
    );
    assert_eq!(boot["result"]["isError"], false, "{boot}");
    let envelope = &boot["result"]["structuredContent"];
    let session = envelope["data"]["sessionId"].as_str().unwrap().to_owned();
    assert!(envelope["data"].get("rules").is_none());
    assert!(envelope["omitted"].get("truncated").is_none());
    let search = server.request(4, "tools/call", json!({"name":"harness_search","arguments":{"session":session,"kind":"text","query":"needle","limit":1}}));
    assert_eq!(search["result"]["isError"], false, "{search}");
    let search = &search["result"]["structuredContent"];
    assert_eq!(search["data"]["matches"].as_array().unwrap().len(), 1);
    assert_eq!(search["omitted"]["matches"], 1);
    assert!(search["data"]["complete"].is_boolean());
    let reference = search["detail"]["ref"].as_str().unwrap().to_owned();
    let full = server.request(
        5,
        "tools/call",
        json!({"name":"harness_detail","arguments":{"ref":search["detail"]["ref"]}}),
    );
    let full: Value = serde_json::from_str(
        full["result"]["structuredContent"]["data"]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(full["matches"].as_array().unwrap().len(), 2);
    let state_path = root
        .path()
        .join(format!(".agents/harness/.state/{session}.json"));
    let saved: Value = serde_json::from_slice(&std::fs::read(state_path).unwrap()).unwrap();
    assert_eq!(saved["version"], 1);
    assert_eq!(saved["id"], session);
    assert_eq!(saved["data"]["phases"].as_array().unwrap().len(), 4);
    drop(server);
    let mut server = Server::start(root.path());
    server.initialize();
    let restored = server.request(
        2,
        "tools/call",
        json!({"name":"harness_bootstrap","arguments":{"session":session}}),
    );
    assert_eq!(restored["result"]["isError"], false, "{restored}");
    assert_eq!(
        restored["result"]["structuredContent"]["data"]["sessionId"],
        session
    );
    let expired = server.request(
        3,
        "tools/call",
        json!({"name":"harness_detail","arguments":{"ref":reference}}),
    );
    assert_eq!(
        expired["result"]["structuredContent"]["diagnostics"][0]["code"],
        "DETAIL"
    );
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[cfg(windows)]
fn mcp_replace_publishes_and_rejects_a_stale_retry_without_node() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("edit.json"), "{\"x\":1}\r\n").unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let read = server.request(
        2,
        "tools/call",
        json!({"name":"harness_read","arguments":{"path":"edit.json","start":1,"end":1}}),
    );
    // The full source digest is retained for stale-write protection.
    assert!(read["result"]["structuredContent"]["data"]["hash"].is_string());
    let hash = json!(harness_domain::hash(b"{\"x\":1}\r\n"));
    let arguments = json!({"path":"edit.json","hash":hash,"oldText":"1","newText":"2"});
    let edit = server.request(
        3,
        "tools/call",
        json!({"name":"harness_replace","arguments":arguments}),
    );
    assert_eq!(edit["result"]["isError"], false, "{edit}");
    assert_eq!(
        edit["result"]["structuredContent"]["data"]["verified"],
        true
    );
    let retry = server.request(
        4,
        "tools/call",
        json!({"name":"harness_replace","arguments":arguments}),
    );
    assert_eq!(
        retry["result"]["structuredContent"]["diagnostics"][0]["code"],
        "STALE"
    );
    assert_eq!(
        std::fs::read(root.path().join("edit.json")).unwrap(),
        b"{\"x\":2}\r\n"
    );
    let multi = server.request(
        5,
        "tools/call",
        json!({"name":"harness_replace","arguments":{
            "path":"edit.json", "hash":harness_domain::hash(b"{\"x\":2}\r\n"),
            "chunks":[{"oldText":"2","newText":"3"},{"oldText":"\"x\"","newText":"\"y\""}]
        }}),
    );
    assert_eq!(multi["result"]["isError"], false, "{multi}");
    assert_eq!(
        std::fs::read(root.path().join("edit.json")).unwrap(),
        b"{\"y\":3}\r\n"
    );
}

#[test]
#[cfg(windows)]
fn mcp_replace_reports_js_error_location_without_publishing_and_accepts_completed_candidate() {
    let root = tempfile::tempdir().unwrap();
    let invalid_source = "const ok=1;\r\nconst emoji = '😀'; const value = 1;\r\n";
    std::fs::write(root.path().join("invalid.js"), invalid_source).unwrap();
    std::fs::write(
        root.path().join("complete.js"),
        "function f() { return TODO",
    )
    .unwrap();
    let mut server = Server::start(root.path());
    server.initialize();

    let invalid_args = json!({"path":"invalid.js","oldText":"value = 1","newText":"value = "});
    let preview = server.request(
        2,
        "tools/call",
        json!({"name":"harness_replace","arguments":{
            "path":"invalid.js","oldText":"value = 1","newText":"value = ","preview":true
        }}),
    );
    assert_eq!(preview["result"]["isError"], false, "{preview}");
    assert_eq!(
        preview["result"]["structuredContent"]["data"]["published"],
        false
    );
    assert_eq!(
        preview["result"]["structuredContent"]["data"]["syntax"]["code"],
        "SYNTAX_ERROR"
    );
    assert!(
        preview["result"]["structuredContent"]["data"]["syntax"]["message"]
            .as_str()
            .unwrap()
            .contains("line 2, column 35")
    );
    assert_eq!(
        std::fs::read(root.path().join("invalid.js")).unwrap(),
        invalid_source.as_bytes()
    );

    let invalid = server.request(
        3,
        "tools/call",
        json!({"name":"harness_replace","arguments":invalid_args}),
    );
    assert_eq!(invalid["result"]["isError"], true, "{invalid}");
    let envelope = &invalid["result"]["structuredContent"];
    assert_eq!(envelope["state"], "error");
    assert_eq!(envelope["data"]["changed"], false);
    assert_eq!(envelope["diagnostics"][0]["code"], "SYNTAX_ERROR");
    assert!(
        envelope["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("line 2, column 35")
    );
    assert_eq!(
        std::fs::read(root.path().join("invalid.js")).unwrap(),
        invalid_source.as_bytes()
    );

    let completed = server.request(
        4,
        "tools/call",
        json!({"name":"harness_replace","arguments":{
            "path":"complete.js","oldText":"TODO","newText":"1; }"
        }}),
    );
    assert_eq!(completed["result"]["isError"], false, "{completed}");
    assert_eq!(
        completed["result"]["structuredContent"]["data"]["verified"],
        true
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("complete.js")).unwrap(),
        "function f() { return 1; }"
    );
}

#[test]
#[cfg(windows)]
fn mcp_move_roundtrip_and_non_overwrite_without_node() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("source.txt"), "original\r\n").unwrap();
    std::fs::write(root.path().join("existing.txt"), "keep").unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let moved = server.request(
        2,
        "tools/call",
        json!({"name":"harness_move", "arguments":{"from":"source.txt","to":"new.txt"}}),
    );
    assert_eq!(moved["result"]["isError"], false, "{moved}");
    assert_eq!(
        moved["result"]["structuredContent"]["data"]["verified"],
        true
    );
    let denied = server.request(
        3,
        "tools/call",
        json!({"name":"harness_move", "arguments":{"from":"new.txt","to":"existing.txt"}}),
    );
    assert_eq!(
        denied["result"]["structuredContent"]["diagnostics"][0]["code"],
        "MOVE_EXISTS"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("new.txt")).unwrap(),
        "original\r\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("existing.txt")).unwrap(),
        "keep"
    );
}

#[test]
#[cfg(windows)]
fn mcp_delete_roundtrip_schema_and_profiles() {
    for profile in ["full", "edit", "investigate"] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("obsolete.txt"), "old\r\n").unwrap();
        let mut server =
            Server::start_with_profile(root.path(), false, false, false, profile, true);
        server.initialize();
        let catalog = server.request(2, "tools/list", json!({}));
        let delete = catalog["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "harness_delete");
        if profile == "investigate" {
            assert!(delete.is_none(), "{catalog}");
            let denied = server.request(
                3,
                "tools/call",
                json!({"name":"harness_delete","arguments":{"path":"obsolete.txt"}}),
            );
            assert!(denied.get("error").is_some(), "{denied}");
            assert!(root.path().join("obsolete.txt").exists());
            continue;
        }
        let schema = &delete.unwrap()["inputSchema"];
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("path"))
        );
        assert!(schema["properties"].get("expectedHash").is_some());
        assert_eq!(schema["additionalProperties"], false);
        let stale = server.request(
            3,
            "tools/call",
            json!({"name":"harness_delete","arguments":{
                "path":"obsolete.txt","expectedHash":harness_domain::hash(b"different")
            }}),
        );
        assert_eq!(
            stale["result"]["structuredContent"]["diagnostics"][0]["code"],
            "STALE"
        );
        assert!(root.path().join("obsolete.txt").exists());
        let result = server.request(
            4,
            "tools/call",
            json!({"name":"harness_delete","arguments":{
                "path":"obsolete.txt","expectedHash":harness_domain::hash(b"old\r\n")
            }}),
        );
        assert_eq!(result["result"]["isError"], false, "{result}");
        assert_eq!(
            result["result"]["structuredContent"]["data"]["verified"],
            true
        );
        assert!(!root.path().join("obsolete.txt").exists());
    }
}

#[test]
#[cfg(windows)]
fn mcp_proposal_prepare_is_additive_and_does_not_publish() {
    for profile in ["full", "edit", "investigate"] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file.txt"), "old").unwrap();
        let mut server =
            Server::start_with_profile(root.path(), false, false, false, profile, true);
        server.initialize();
        let list = server.request(2, "tools/list", json!({}));
        let proposal = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "harness_proposal");
        if profile == "investigate" {
            assert!(proposal.is_none());
            let denied = server.request(
                3,
                "tools/call",
                json!({"name":"harness_proposal","arguments":{
                    "action":"prepare","path":"file.txt","content":"new"
                }}),
            );
            assert!(denied.get("error").is_some(), "{denied}");
            assert!(!root.path().join(".agents/harness/proposals").exists());
            continue;
        }
        let schema = &proposal.unwrap()["inputSchema"];
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("content").is_some());
        let result = server.request(3, "tools/call", json!({"name":"harness_proposal","arguments":{
            "action":"prepare","path":"file.txt","content":"new","expectedHash":harness_domain::hash(b"old")
        }}));
        assert_eq!(result["result"]["isError"], false, "{result}");
        let data = &result["result"]["structuredContent"]["data"];
        assert_eq!(data["stored"], true);
        assert_eq!(data["published"], false);
        assert_eq!(data["changed"], false);
        assert!(
            root.path()
                .join(data["storagePath"].as_str().unwrap())
                .exists()
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("file.txt")).unwrap(),
            "old"
        );
        let unsupported = server.request(
            4,
            "tools/call",
            json!({"name":"harness_proposal","arguments":{
                "action":"accept","path":"file.txt","content":"new"
            }}),
        );
        assert!(
            unsupported["error"].is_object() || unsupported["result"]["isError"] == true,
            "{unsupported}"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("file.txt")).unwrap(),
            "old"
        );
    }
}

#[test]
fn mcp_html_parser_reads_elements_and_preserves_embedded_css_selectors() {
    let root = tempfile::tempdir().unwrap();
    let button = "<button id='save' [disabled]=\"busy()\">{{ label }}</button>";
    let source = format!("<style>.action {{ color: red; }}</style>\r\n{button}");
    std::fs::write(root.path().join("page.html"), &source).unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let out = server.request(
        2,
        "tools/call",
        json!({"name":"harness_symbols","arguments":{
            "action":"outline","path":"page.html","maxChars":16000
        }}),
    );
    assert_eq!(out["result"]["isError"], false, "{out}");
    assert_eq!(
        out["result"]["structuredContent"]["data"]["structure"]["parser"],
        "tl"
    );
    let result = server.request(
        3,
        "tools/call",
        json!({"name":"harness_symbols","arguments":{
            "action":"read","path":"page.html","symbol":"html:#save","view":"full"
        }}),
    );
    assert_eq!(result["result"]["isError"], false, "{result}");
    let data = &result["result"]["structuredContent"]["data"];
    assert_eq!(data["text"], button);
    assert!(
        data["symbol"]["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "[disabled]=\"busy()\"")
    );
    let css = server.request(
        4,
        "tools/call",
        json!({"name":"harness_symbols","arguments":{
            "action":"read","path":"page.html","symbol":".action","view":"full"
        }}),
    );
    assert_eq!(
        css["result"]["structuredContent"]["data"]["text"],
        ".action { color: red; }"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("page.html")).unwrap(),
        source
    );
}

#[test]
fn unified_git_mcp_preserves_status_diff_and_batch() {
    let root = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let mut command = Command::new("git");
        for (key, _) in std::env::vars_os() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                command.env_remove(key);
            }
        }
        let output = command
            .current_dir(root.path())
            .args([
                "-c",
                "core.hooksPath=.no-hooks",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git fixture: {args:?}");
    };
    git(&["init", "--quiet"]);
    std::fs::write(root.path().join("tracked.txt"), "before\n").unwrap();
    git(&["add", "tracked.txt"]);
    git(&[
        "-c",
        "user.name=Harness Test",
        "-c",
        "user.email=harness@example.invalid",
        "commit",
        "--quiet",
        "-m",
        "fixture",
    ]);
    std::fs::write(root.path().join("tracked.txt"), "staged\n").unwrap();
    git(&["add", "tracked.txt"]);
    std::fs::write(root.path().join("tracked.txt"), "worktree\n").unwrap();
    std::fs::write(root.path().join("untracked.txt"), "untracked\n").unwrap();
    let mut server = Server::start_with_runtime(root.path(), false, true);
    server.initialize();
    let status = server.request(2, "tools/call", json!({"name":"harness_git","arguments":{"action":"status","includeUntracked":false,"limit":1}}));
    let status = &status["result"]["structuredContent"];
    assert_eq!(status["state"], "ok", "{status}");
    assert_eq!(status["data"]["entries"].as_array().unwrap().len(), 1);
    for (id, staged, present, absent) in [
        (3, false, "+worktree", "+staged"),
        (4, true, "+staged", "+worktree"),
    ] {
        let diff = server.request(id, "tools/call", json!({"name":"harness_git","arguments":{"action":"diff","path":"tracked.txt","staged":staged}}));
        let envelope = &diff["result"]["structuredContent"];
        assert_eq!(envelope["state"], "ok", "{diff}");
        let content = envelope["data"].to_string();
        assert!(content.contains(present), "{diff}");
        assert!(!content.contains(absent), "{diff}");
    }
    let batch = server.request(
        5,
        "tools/call",
        json!({"name":"harness_batch","arguments":{"jobs":[
            {"tool":"harness_git","arguments":{"action":"status"}},
            {"tool":"harness_git","arguments":{"action":"diff","path":"tracked.txt"}}
        ]}}),
    );
    assert_eq!(
        batch["result"]["structuredContent"]["state"], "ok",
        "{batch}"
    );
    let invalid = server.request(
        6,
        "tools/call",
        json!({"name":"harness_git","arguments":{"action":"status","path":"tracked.txt"}}),
    );
    assert!(
        invalid.get("error").is_some() || invalid["result"]["isError"] == true,
        "{invalid}"
    );
}

#[test]
fn rust_symbol_navigation_works_over_stdio_without_node() {
    let root = tempfile::tempdir().unwrap();
    let source = "// 🙂\r\npub fn hello() -> i32 { 1 }\r\n";
    std::fs::write(root.path().join("sample.rs"), source).unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let outline = server.request(
        2,
        "tools/call",
        json!({"name":"harness_symbols","arguments":{"action":"outline","path":"sample.rs"}}),
    );
    assert_eq!(outline["result"]["isError"], false, "{outline}");
    let id = outline["result"]["structuredContent"]["data"]["items"][0]["symbolId"].clone();
    let full = server.request(3,"tools/call",json!({"name":"harness_symbols","arguments":{"action":"read","path":"sample.rs","symbol":"hello","symbolId":id}}));
    assert_eq!(full["result"]["isError"], false, "{full}");
    assert_eq!(
        full["result"]["structuredContent"]["data"]["text"],
        "pub fn hello() -> i32 { 1 }"
    );
    // Anchors and full-file hash are display noise for the model now; the
    // lean text channel carries the source numbered instead.
    let signature = server.request(4,"tools/call",json!({"name":"harness_symbols","arguments":{"action":"read","path":"sample.rs","symbol":"hello","symbolId":id,"view":"signature"}}));
    assert_eq!(
        signature["result"]["structuredContent"]["data"]["editReady"],
        false
    );
    assert!(
        signature["result"]["structuredContent"]["data"]
            .get("text")
            .is_none()
    );
    let batch = server.request(40, "tools/call", json!({"name":"harness_batch","arguments":{"jobs":[
        {"tool":"harness_symbols","arguments":{"action":"outline","path":"sample.rs"}},
        {"tool":"harness_symbols","arguments":{"action":"read","path":"sample.rs","symbol":"hello","view":"full"}}
    ]}}));
    let children = &batch["result"]["structuredContent"]["data"]["results"];
    assert_eq!(children[0]["state"], "ok", "{batch}");
    assert_eq!(children[1]["state"], "ok", "{batch}");
    assert_eq!(
        children[1]["data"]["text"],
        full["result"]["structuredContent"]["data"]["text"]
    );
    assert_eq!(
        children[1]["data"]["hash"],
        full["result"]["structuredContent"]["data"]["hash"]
    );
    #[cfg(windows)]
    {
        let data = &full["result"]["structuredContent"]["data"];
        let edited = server.request(5,"tools/call",json!({"name":"harness_replace","arguments":{"path":"sample.rs","hash":data["hash"],"oldText":data["text"],"newText":"pub fn hello() -> i32 { 2 }"}}));
        assert_eq!(edited["result"]["isError"], false, "{edited}");
        let stale = server.request(6,"tools/call",json!({"name":"harness_symbols","arguments":{"action":"read","path":"sample.rs","symbol":"hello","symbolId":id}}));
        assert_eq!(
            stale["result"]["structuredContent"]["diagnostics"][0]["code"],
            "STALE_SYMBOL"
        );
    }
}

#[test]
fn workflow_roundtrip_and_task_timeline_work_without_node() {
    let root = tempfile::tempdir().unwrap();
    let mut server = Server::start(root.path());
    server.initialize();
    let boot = server.request(
        2,
        "tools/call",
        json!({"name":"harness_bootstrap","arguments":{}}),
    );
    let session = boot["result"]["structuredContent"]["data"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let updated = server.request(
        3,
        "tools/call",
        json!({"name":"harness_session","arguments":{"action":"tasks","session":session,"phase":"analysis","status":"in_progress"}}),
    );
    assert_eq!(updated["result"]["isError"], false, "{updated}");
    assert_eq!(
        updated["result"]["structuredContent"]["data"]["persisted"],
        true
    );
    let declared = server.request(
        4,
        "tools/call",
        json!({"name":"harness_session","arguments":{"action":"checkpoint","session":session,"goal":"seguir Rust 🙂","decisions":["nativo"],"pending":["vigencia"]}}),
    );
    assert_eq!(declared["result"]["isError"], false, "{declared}");
    let timeline = server.request(
        5,
        "tools/call",
        json!({"name":"harness_session","arguments":{"action":"tasks","session":session,"limit":1}}),
    );
    assert_eq!(
        timeline["result"]["structuredContent"]["data"]["events"][0]["type"],
        "plan_created"
    );
    assert_eq!(
        timeline["result"]["structuredContent"]["data"]["nextAfter"],
        1
    );
    drop(server);
    let mut server = Server::start(root.path());
    server.initialize();
    let restored = server.request(
        2,
        "tools/call",
        json!({"name":"harness_session","arguments":{"action":"status","session":session}}),
    );
    assert_eq!(restored["result"]["isError"], false, "{restored}");
    assert_eq!(
        restored["result"]["structuredContent"]["data"]["declaredByAgent"]["goal"],
        "seguir Rust 🙂"
    );
    assert_eq!(
        restored["result"]["structuredContent"]["data"]["tasks"]["active"],
        "analysis"
    );
    assert_eq!(
        restored["result"]["structuredContent"]["data"]["freshness"]["available"],
        false
    );
}

#[test]
fn oversized_stdio_message_closes_connection() {
    let root = tempfile::tempdir().unwrap();
    let mut server = Server::start(root.path());
    let input = server.child.stdin.as_mut().unwrap();
    let _ = input.write_all(&vec![b'x'; 128 * 1024 + 1]);
    let _ = input.flush();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if server.child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "oversized input did not close MCP transport"
        );
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

#[test]
fn mcp_handshake_discovery_bootstrap_and_safe_read() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join(".agents/rules")).unwrap();
    std::fs::write(
        root.path().join(".agents/rules/00-mandatory-tools.md"),
        "Reglas de prueba",
    )
    .unwrap();
    std::fs::write(root.path().join("example.txt"), "hola\r\nRust\r\n").unwrap();
    let mut server = Server::start(root.path());
    let init = server.request(1, "initialize", json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"compatibility-test","version":"1"}}));
    assert!(init.get("error").is_none(), "{init}");
    server.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    let listed = server.request(2, "tools/list", json!({}));
    let names: Vec<_> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let mut actual = names.clone();
    actual.sort();
    let mut expected = harness_domain::tool_contract::tool_names("rust");
    expected.sort();
    assert_eq!(
        actual,
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );
    for name in ["harness_explore", "harness_data_query"] {
        let tool = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap();
        let shared = &harness_domain::tool_contract::manifest()["schemas"][name];
        for field in ["properties", "required", "additionalProperties", "oneOf"] {
            assert_eq!(
                tool["inputSchema"][field], shared[field],
                "schema drift: {name}.{field}"
            );
        }
    }
    assert!(names.contains(&"harness_session"));
    assert!(!names.contains(&"harness_handover"));
    assert!(names.contains(&"harness_doctor"));
    assert!(!names.contains(&"harness_diagnostics"));
    assert!(!names.contains(&"harness_logs"));
    assert!(!names.contains(&"harness_process_status"));
    assert!(names.contains(&"harness_data_query"));
    assert!(names.contains(&"harness_explore"));
    assert!(names.contains(&"harness_config") && names.contains(&"harness_external_scope"));
    assert!(names.contains(&"harness_sql"));
    assert!(!names.iter().any(|name| name.starts_with("harness_sql_")));
    assert!(names.contains(&"harness_xlsx"));
    assert!(!names.iter().any(|name| name.starts_with("harness_xlsx_")));
    assert!(names.contains(&"harness_svg"));
    let sizes=server.request(27,"tools/call",json!({"name":"harness_search","arguments":{"kind":"file_size","scope":"example.txt","minLines":1}}));
    assert_eq!(
        sizes["result"]["structuredContent"]["state"], "ok",
        "{sizes}"
    );
    assert_eq!(
        sizes["result"]["structuredContent"]["data"]["files"][0]["lines"],
        2
    );
    let logs = server.request(
        25,
        "tools/call",
        json!({"name":"harness_doctor","arguments":{"action":"logs","limit":5}}),
    );
    assert_eq!(logs["result"]["structuredContent"]["state"], "ok", "{logs}");
    assert!(names.contains(&"harness_batch"));
    let batch = server.request(24,"tools/call",json!({"name":"harness_batch","arguments":{"jobs":[{"tool":"harness_read","arguments":{"path":"example.txt","start":1,"end":1}}]}}));
    assert_eq!(
        batch["result"]["structuredContent"]["state"], "ok",
        "{batch}"
    );
    assert!(names.contains(&"harness_doctor"));
    let doctor = server.request(
        23,
        "tools/call",
        json!({"name":"harness_doctor","arguments":{"action":"env","connection":true}}),
    );
    let diagnosis = &doctor["result"]["structuredContent"];
    assert_eq!(diagnosis["state"], "ok", "{doctor}");
    assert_eq!(diagnosis["data"]["versionsRequested"], false);
    let auto_sync = diagnosis["data"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["id"] == "index-auto-sync")
        .unwrap();
    assert_eq!(auto_sync["details"]["enabled"], true);
    assert!(root.path().join(".codegraph/codegraph.db").exists());
    assert_eq!(diagnosis["data"]["allToolsVerified"], false);
    assert_eq!(diagnosis["data"]["mcpRegistry"]["status"], "verified");
    let coverage = diagnosis["data"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["id"] == "functional-coverage")
        .unwrap();
    assert_eq!(coverage["status"], "warning");
    for name in [
        "tool:cargo",
        "tool:rustc",
        "tool:git",
        "tool:go",
        "tool:python",
        "tool:node",
        "tool:dotnet",
    ] {
        let tool = diagnosis["data"]["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == name)
            .unwrap();
        assert_eq!(tool["located"], false, "{doctor}");
        assert_eq!(tool["versionChecked"], false);
    }
    assert!(names.contains(&"harness_git"));
    assert!(!names.contains(&"harness_git_status") && !names.contains(&"harness_git_diff"));
    assert!(names.contains(&"harness_session"));
    assert!(!names.contains(&"harness_tasks") && !names.contains(&"harness_checkpoint"));
    assert!(names.contains(&"harness_symbols"));
    assert!(!names.contains(&"harness_outline") && !names.contains(&"harness_read_symbol"));
    assert!(names.contains(&"harness_replace"));
    assert!(names.contains(&"harness_verify"));
    let nonrepo = server.request(
        22,
        "tools/call",
        json!({"name":"harness_git","arguments":{"action":"status"}}),
    );
    assert_eq!(
        nonrepo["result"]["structuredContent"]["diagnostics"][0]["code"],
        "GIT_REPO"
    );
    let unsupported = server.request(
        20,
        "tools/call",
        json!({"name":"harness_verify","arguments":{"language":"unknown-language"}}),
    );
    assert_eq!(unsupported["result"]["structuredContent"]["state"], "error");
    assert_eq!(
        unsupported["result"]["structuredContent"]["diagnostics"][0]["code"],
        "UNSUPPORTED"
    );
    let missing_ts_scope = server.request(
        23,
        "tools/call",
        json!({"name":"harness_verify","arguments":{"language":"typescript"}}),
    );
    assert_eq!(
        missing_ts_scope["result"]["structuredContent"]["diagnostics"][0]["code"],
        "VERIFY_SCOPE"
    );
    std::fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname='no-cargo'\nversion='0.1.0'\n",
    )
    .unwrap();
    let missing = server.request(
        21,
        "tools/call",
        json!({"name":"harness_verify","arguments":{"language":"rust"}}),
    );
    assert_eq!(missing["result"]["structuredContent"]["state"], "error");
    assert_eq!(
        missing["result"]["structuredContent"]["diagnostics"][0]["code"],
        "PROCESS_EXECUTABLE"
    );
    assert!(names.contains(&"harness_bootstrap") && names.contains(&"harness_read"));
    let boot = server.request(
        3,
        "tools/call",
        json!({"name":"harness_bootstrap","arguments":{}}),
    );
    assert_eq!(boot["result"]["isError"], false, "{boot}");
    assert!(boot["result"]["structuredContent"]["data"].get("rules").is_none());
    let session = boot["result"]["structuredContent"]["data"]["sessionId"]
        .as_str()
        .unwrap();
    assert_eq!(
        boot["result"]["structuredContent"]["data"]["session"],
        session
    );
    let alias_boot = server.request(
        93,
        "tools/call",
        json!({"name":"harness_bootstrap","arguments":{"sessionId":session}}),
    );
    assert_eq!(alias_boot["result"]["isError"], false, "{alias_boot}");
    assert_eq!(
        alias_boot["result"]["structuredContent"]["data"]["session"],
        session
    );
    let alias_explore = server.request(
        94,
        "tools/call",
        json!({"name":"harness_explore","arguments":{"sessionId":session,"query":"main"}}),
    );
    // The fixture has a bootstrap-initialized empty index: expect the plain
    // application outcome (unresolved target), not a JSON-RPC argument rejection.
    assert!(alias_explore.get("error").is_none(), "{alias_explore}");
    assert_eq!(
        direct_tool_payload(&alias_explore)["unresolved"][0]["code"],
        "TARGET_NOT_RESOLVED"
    );
    let read = server.request(4, "tools/call", json!({"name":"harness_read","arguments":{"session":session,"path":"example.txt","start":1,"end":2}}));
    assert_eq!(
        read["result"]["structuredContent"]["data"]["text"],
        "hola\r\nRust"
    );
    let id = read["result"]["structuredContent"]["requestId"]
        .as_str()
        .unwrap();
    let traced = server.request(
        26,
        "tools/call",
        json!({"name":"harness_doctor","arguments":{"action":"logs","requestId":id}}),
    );
    assert_eq!(
        traced["result"]["structuredContent"]["data"]["events"][0]["requestId"],
        id
    );
    // The reading channel is now lean markdown; the canonical envelope lives
    // only in structuredContent (its requestId matched the trace above).
    let repeated = server.request(
        5,
        "tools/call",
        json!({"name":"harness_bootstrap","arguments":{"session":session}}),
    );
    assert!(repeated["result"]["structuredContent"]["data"].get("rulesUnchanged").is_none());
    assert_eq!(repeated["result"]["structuredContent"]["data"]["capabilitiesUnchanged"], true);
    let denied = server.request(
        6,
        "tools/call",
        json!({"name":"harness_read","arguments":{"path":"../outside","start":1,"end":1}}),
    );
    assert_eq!(denied["result"]["isError"], true);
    assert_eq!(
        denied["result"]["structuredContent"]["diagnostics"][0]["code"],
        "PATH"
    );
    let invalid_session = server.request(7, "tools/call", json!({"name":"harness_read","arguments":{"session":"missing","path":"example.txt","start":1,"end":1}}));
    assert_eq!(
        invalid_session["result"]["structuredContent"]["diagnostics"][0]["code"],
        "SESSION"
    );
    let unknown_arg = server.request(8, "tools/call", json!({"name":"harness_read","arguments":{"path":"example.txt","start":1,"end":1,"externalScope":"unsupported"}}));
    assert!(
        unknown_arg.get("error").is_some() || unknown_arg["result"]["isError"] == true,
        "{unknown_arg}"
    );
}
