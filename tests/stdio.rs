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
                    "harness_read_symbol",
                    json!({"session":session,"path":"crates/api/src/main.rs","symbol":"main"}),
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
                let envelope = &response["result"]["structuredContent"];
                assert_eq!(envelope["state"], "ok", "{tool}: {envelope}");
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
        let envelope = &result["result"]["structuredContent"];
        assert_eq!(envelope["state"], "ok", "{envelope}");
        let request_id = envelope["requestId"].as_str().unwrap();
        let logs = server.request(4, "tools/call", json!({"name":"harness_logs","arguments":{"requestId":request_id,"limit":10,"session":session}}));
        assert_eq!(logs["result"]["structuredContent"]["state"], "ok");
        let events = logs["result"]["structuredContent"]["data"]["events"]
            .as_array()
            .unwrap();
        let completed = events
            .iter()
            .find(|e| e["requestId"] == request_id && e["kind"] == "completed")
            .expect("correlated terminal log");
        assert_eq!(completed["state"], "ok", "{completed}");
        let data = &envelope["data"];
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
                "structuredBytes":serde_json::to_vec(envelope).unwrap().len(),
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
        let envelope = &response["result"]["structuredContent"];
        assert_eq!(envelope["state"], "ok", "{envelope}");
        let data = &envelope["data"];
        let graph = json!({"nodes":data["graph"]["nodes"],"edges":data["graph"]["edges"]});
        if let Some(original) = &original_graph {
            assert_eq!(original, &graph);
        }
        original_graph = Some(graph);
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        if context {
            assert!(text.starts_with("## Graph context"));
            assert!(text.find("sourceStatus").unwrap() < text.find("**Source Code**").unwrap());
            assert!(
                text.find("**Call paths / blast radius").unwrap()
                    < text.find("**Source Code**").unwrap()
            );
            assert!(text.contains("not_certified"));
            assert!(text.contains(envelope["requestId"].as_str().unwrap()));
            assert!(text.contains("sourceStatus"));
        } else {
            assert_eq!(serde_json::from_str::<Value>(text).unwrap(), *envelope);
        }
        println!(
            "{}",
            json!({"presentation":if context {"context"} else {"legacy"},
            "wireBytes":serde_json::to_vec(&response).unwrap().len(),
            "textChannelBytes":text.len(),"structuredBytes":serde_json::to_vec(envelope).unwrap().len(),
            "nodeCount":data["graph"]["nodes"].as_array().unwrap().len(),
            "edgeCount":data["graph"]["edges"].as_array().unwrap().len(),
            "completeBodies":data["definitions"].as_array().unwrap().iter().filter(|d| d["sourceStatus"]=="complete").count()})
        );
    }
}

#[test]
fn tool_profiles_filter_registry_and_enforce_direct_and_batch_calls() {
    for (profile, count) in [("investigate", 13), ("edit", 16)] {
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
        assert_eq!(data["toolProfile"], profile);
        let mut actual: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        let mut declared: Vec<_> = data["supportedTools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        actual.sort();
        declared.sort();
        assert_eq!(actual, declared);
        let blocked = server.request(
            4,
            "tools/call",
            json!({"name":"harness_sql_status","arguments":{}}),
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
            sync["result"]["structuredContent"]["diagnostics"][0]["code"],
            "TOOL_PROFILE"
        );
        let batch = server.request(
            6,
            "tools/call",
            json!({"name":"harness_batch","arguments":{"jobs":[
                {"tool":"harness_read","arguments":{"path":"example.txt","start":1,"end":1}},
                {"tool":"harness_sql_status","arguments":{}}
            ]}}),
        );
        assert_eq!(
            batch["result"]["structuredContent"]["diagnostics"][0]["code"],
            "TOOL_PROFILE"
        );
        let logs = server.request(
            7,
            "tools/call",
            json!({"name":"harness_logs","arguments":{"tool":"harness_read","limit":10}}),
        );
        assert!(
            logs["result"]["structuredContent"]["data"]["events"]
                .as_array()
                .unwrap()
                .is_empty()
        );
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
    assert_eq!(tools.len(), 30);
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
            .env("HARNESS_MCP_EAGER_TOOLS", if eager { "1" } else { "0" });
        if context {
            command.env("HARNESS_MCP_PRESENTATION", "context");
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
    let result = &result["result"]["structuredContent"];
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(
        result["data"]["graph"]["edges"].as_array().unwrap().len(),
        2
    );
    assert_eq!(result["data"]["focus"]["status"], "resolved");
    let deeper = server.request(3, "tools/call", json!({"name":"harness_explore","arguments":{"query":"target","direction":"outbound","maxDepth":2}}));
    let deeper = &deeper["result"]["structuredContent"];
    assert_eq!(deeper["state"], "ok", "{deeper}");
    assert_eq!(
        deeper["data"]["graph"]["traversal"]["nodeDepths"]["c"]["outbound"],
        2
    );
    let matching = server.request(4, "tools/call", json!({"name":"harness_explore","arguments":{"query":"target","requireMatchingFiles":true}}));
    assert_eq!(
        matching["result"]["structuredContent"]["data"]["fileFreshness"]["status"],
        "matches_observed_files"
    );
    std::fs::write(root.path().join("lib.rs"), "pub fn target() {}\n").unwrap();
    let stale = server.request(5, "tools/call", json!({"name":"harness_explore","arguments":{"query":"target","requireMatchingFiles":true}}));
    assert_eq!(
        stale["result"]["structuredContent"]["diagnostics"][0]["code"],
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
    let result = server.request(
        2,
        "tools/call",
        json!({"name":"harness_verify","arguments":{
            "language":"javascript","scope":"example.js","action":"build"
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
        json!({"name":"harness_logs","arguments":{"requestId":id}}),
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
    let reference = envelope["detail"]["ref"].as_str().unwrap().to_owned();
    assert_eq!(envelope["omitted"]["truncated"], true);
    assert_eq!(envelope["detail"]["complete"], true);
    let mut text = String::new();
    let mut offset = 0;
    loop {
        let detail = server.request(3, "tools/call", json!({"name":"harness_detail","arguments":{"session":session,"ref":reference,"offset":offset,"maxChars":137}}));
        assert_eq!(detail["result"]["isError"], false, "{detail}");
        let data = &detail["result"]["structuredContent"]["data"];
        text.push_str(data["text"].as_str().unwrap());
        if data["nextOffset"].is_null() {
            break;
        }
        offset = data["nextOffset"].as_u64().unwrap();
    }
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["rules"],
        rules
    );
    let search = server.request(4, "tools/call", json!({"name":"harness_search","arguments":{"session":session,"kind":"text","query":"needle","limit":1}}));
    assert_eq!(search["result"]["isError"], false, "{search}");
    let search = &search["result"]["structuredContent"];
    assert_eq!(search["data"]["matches"].as_array().unwrap().len(), 1);
    assert_eq!(search["omitted"]["matches"], 1);
    assert_eq!(search["data"]["complete"], true);
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
        restored["result"]["structuredContent"]["data"]["sessionRestored"],
        true
    );
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
    let hash = read["result"]["structuredContent"]["data"]["hash"].clone();
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
        json!({"name":"harness_outline","arguments":{"path":"sample.rs"}}),
    );
    assert_eq!(outline["result"]["isError"], false, "{outline}");
    let id = outline["result"]["structuredContent"]["data"]["items"][0]["symbolId"].clone();
    let full = server.request(3,"tools/call",json!({"name":"harness_read_symbol","arguments":{"path":"sample.rs","symbol":"hello","symbolId":id}}));
    assert_eq!(full["result"]["isError"], false, "{full}");
    assert_eq!(
        full["result"]["structuredContent"]["data"]["text"],
        "pub fn hello() -> i32 { 1 }"
    );
    assert_eq!(
        full["result"]["structuredContent"]["data"]["anchors"]["before"]["text"],
        "// 🙂"
    );
    let signature = server.request(4,"tools/call",json!({"name":"harness_read_symbol","arguments":{"path":"sample.rs","symbol":"hello","symbolId":id,"view":"signature"}}));
    assert_eq!(
        signature["result"]["structuredContent"]["data"]["editReady"],
        false
    );
    assert!(
        signature["result"]["structuredContent"]["data"]
            .get("text")
            .is_none()
    );
    #[cfg(windows)]
    {
        let data = &full["result"]["structuredContent"]["data"];
        let edited = server.request(5,"tools/call",json!({"name":"harness_replace","arguments":{"path":"sample.rs","hash":data["hash"],"oldText":data["text"],"newText":"pub fn hello() -> i32 { 2 }"}}));
        assert_eq!(edited["result"]["isError"], false, "{edited}");
        let stale = server.request(6,"tools/call",json!({"name":"harness_read_symbol","arguments":{"path":"sample.rs","symbol":"hello","symbolId":id}}));
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
    let updated=server.request(3,"tools/call",json!({"name":"harness_tasks","arguments":{"session":session,"action":"update","phase":"analysis","status":"in_progress"}}));
    assert_eq!(updated["result"]["isError"], false, "{updated}");
    assert_eq!(
        updated["result"]["structuredContent"]["data"]["persisted"],
        true
    );
    let declared=server.request(4,"tools/call",json!({"name":"harness_checkpoint","arguments":{"session":session,"goal":"seguir Rust 🙂","decisions":["nativo"],"pending":["vigencia"]}}));
    assert_eq!(declared["result"]["isError"], false, "{declared}");
    let timeline=server.request(5,"tools/call",json!({"name":"harness_tasks","arguments":{"session":session,"action":"timeline","limit":1}}));
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
        json!({"name":"harness_checkpoint","arguments":{"session":session}}),
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
    assert_eq!(names.len(), 30);
    let mut actual = names.clone();
    actual.sort();
    let expected = harness_domain::tool_contract::tool_names("rust");
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
    assert!(names.contains(&"harness_handover"));
    assert!(names.contains(&"harness_process_status"));
    assert!(names.contains(&"harness_diagnostics"));
    assert!(names.contains(&"harness_data_query"));
    assert!(names.contains(&"harness_explore"));
    assert!(names.contains(&"harness_config") && names.contains(&"harness_external_scope"));
    assert!(names.contains(&"harness_logs"));
    assert!(names.contains(&"harness_sql_status") && names.contains(&"harness_sql_search"));
    assert!(names.contains(&"harness_sql_describe") && names.contains(&"harness_sql_query"));
    assert!(names.contains(&"harness_xlsx_inspect") && names.contains(&"harness_xlsx_read"));
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
        json!({"name":"harness_logs","arguments":{"limit":5}}),
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
        json!({"name":"harness_doctor","arguments":{"connection":true}}),
    );
    let diagnosis = &doctor["result"]["structuredContent"];
    assert_eq!(diagnosis["state"], "ok", "{doctor}");
    assert_eq!(diagnosis["data"]["versionsRequested"], false);
    assert_eq!(diagnosis["data"]["counts"]["warning"], 8);
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
    assert!(names.contains(&"harness_git_status") && names.contains(&"harness_git_diff"));
    assert!(names.contains(&"harness_tasks") && names.contains(&"harness_checkpoint"));
    assert!(names.contains(&"harness_outline") && names.contains(&"harness_read_symbol"));
    assert!(names.contains(&"harness_replace"));
    assert!(names.contains(&"harness_verify"));
    let nonrepo = server.request(
        22,
        "tools/call",
        json!({"name":"harness_git_status","arguments":{}}),
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
    assert_eq!(
        boot["result"]["structuredContent"]["data"]["rules"],
        "Reglas de prueba"
    );
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
    // No index in this fixture: an application error, not a JSON-RPC argument rejection.
    assert!(alias_explore.get("error").is_none(), "{alias_explore}");
    assert_eq!(
        alias_explore["result"]["structuredContent"]["state"],
        "error"
    );
    let read = server.request(4, "tools/call", json!({"name":"harness_read","arguments":{"session":session,"path":"example.txt","start":1,"end":2}}));
    assert_eq!(
        read["result"]["structuredContent"]["data"]["text"],
        "hola\r\nRust"
    );
    let envelope: Value =
        serde_json::from_str(read["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let id = read["result"]["structuredContent"]["requestId"]
        .as_str()
        .unwrap();
    let traced = server.request(
        26,
        "tools/call",
        json!({"name":"harness_logs","arguments":{"requestId":id}}),
    );
    assert_eq!(
        traced["result"]["structuredContent"]["data"]["events"][0]["requestId"],
        id
    );
    assert_eq!(envelope, read["result"]["structuredContent"]);
    let repeated = server.request(
        5,
        "tools/call",
        json!({"name":"harness_bootstrap","arguments":{"session":session}}),
    );
    assert_eq!(
        repeated["result"]["structuredContent"]["data"]["rulesUnchanged"],
        true
    );
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
