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

impl Server {
    fn start(root: &std::path::Path) -> Self {
        Self::start_with_logging(root, false)
    }
    fn start_with_logging(root: &std::path::Path, persist: bool) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-harness"));
        command.env("PATH", "").arg("--root").arg(root);
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
    assert_eq!(names.len(), 25);
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
    assert_eq!(diagnosis["data"]["counts"]["warning"], 7);
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
