//! Exercise the workspace through the same stdio boundary used by MCP clients.
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

struct Server {
    child: Child,
    messages: mpsc::Receiver<String>,
    id: u64,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn start(manifest: &std::path::Path) -> Self {
        let executable = std::env::var_os("HARNESS_NATIVE_BINARY")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_agent-harness").into());
        let mut child = Command::new(executable)
            .arg("--workspace-config")
            .arg(manifest)
            .env("HARNESS_INDEX_WATCH", "0")
            .env("HARNESS_TOOL_PROFILE", "full")
            .env("HARNESS_MCP_EAGER_TOOLS", "1")
            .env_remove("HARNESS_MCP_PRESENTATION")
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
        let mut server = Self {
            child,
            messages,
            id: 0,
        };
        let init = server.request(
            "initialize",
            json!({
                "protocolVersion":"2025-11-25", "capabilities":{},
                "clientInfo":{"name":"workspace-test","version":"1"}
            }),
        );
        assert!(init.get("error").is_none(), "{init}");
        server.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        server
    }

    fn send(&mut self, message: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        self.send(json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params}));
        loop {
            let line = self
                .messages
                .recv_timeout(Duration::from_secs(20))
                .expect("MCP response timeout");
            let response: Value = serde_json::from_str(&line).expect("stdout must be JSON-RPC");
            if response["id"] == self.id {
                return response;
            }
        }
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let response = self.request("tools/call", json!({"name":name,"arguments":arguments}));
        assert!(response.get("error").is_none(), "{response}");
        let envelope = response["result"]["structuredContent"].clone();
        assert_eq!(envelope["state"], "ok", "{response}");
        envelope
    }
}

#[test]
fn workspace_stdio_reloads_projects_keeps_session_and_preserves_last_valid_config() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["backend", "frontend"] {
        std::fs::create_dir(temp.path().join(name)).unwrap();
        std::fs::write(temp.path().join(name).join("sample.txt"), name).unwrap();
    }
    let manifest = temp.path().join("workspace.json");
    std::fs::write(&manifest, r#"{"projects":{"backend":"backend"}}"#).unwrap();
    let mut server = Server::start(&manifest);
    let bootstrap = server.call("harness_bootstrap", json!({}));
    let session = bootstrap["data"]["session"].as_str().unwrap();
    std::fs::write(temp.path().join("backend/sample-extra.txt"), "extra").unwrap();
    let search = server.call(
        "harness_search",
        json!({"kind":"files","query":"sample","limit":1,"session":session}),
    );
    let detail = search["detail"]["ref"].clone();
    assert!(detail.is_string(), "{search}");
    assert!(
        temp.path()
            .join("backend/.agents/skills/harness-workflow/SKILL.md")
            .is_file()
    );
    std::fs::write(
        &manifest,
        r#"{"projects":{"backend":"backend","frontend":"frontend"}}"#,
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    let read = server.call(
        "harness_read",
        json!({"path":"frontend:/sample.txt","start":1,"end":1,"session":session}),
    );
    assert_eq!(read["data"]["text"], "frontend");
    server.call("harness_detail", json!({"ref":detail,"session":session}));
    assert!(
        temp.path()
            .join("frontend/.agents/skills/harness-workflow/SKILL.md")
            .is_file()
    );
    assert_eq!(
        serde_json::from_slice::<Value>(
            &std::fs::read(
                temp.path()
                    .join("frontend/.agents/harness-skills-sync.json")
            )
            .unwrap()
        )
        .unwrap()["auto_sync"],
        true
    );
    std::fs::write(&manifest, b"{ incomplete").unwrap();
    std::thread::sleep(Duration::from_millis(400));
    server.call(
        "harness_read",
        json!({"path":"backend:/sample.txt","start":1,"end":1,"session":session}),
    );
    let rejected = server.call("harness_bootstrap", json!({"session":session}));
    assert!(
        rejected["data"]["workspaceReload"]["lastError"].is_string(),
        "{rejected}"
    );
    std::fs::write(&manifest, r#"{"projects":{"frontend":"frontend"}}"#).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    let response = server.request("tools/call", json!({"name":"harness_read","arguments":{"path":"backend:/sample.txt","start":1,"end":1,"session":session}}));
    assert_eq!(response["result"]["structuredContent"]["state"], "error");
    server.call(
        "harness_read",
        json!({"path":"frontend:/sample.txt","start":1,"end":1,"session":session}),
    );
    std::fs::write(
        &manifest,
        r#"{"projects":{"backend":"backend","frontend":"frontend"}}"#,
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    server.call(
        "harness_read",
        json!({"path":"backend:/sample.txt","start":1,"end":1,"session":session}),
    );
}

#[test]
fn workspace_reload_retains_old_jobs_and_rejects_new_work_on_removed_projects() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["backend", "frontend"] {
        std::fs::create_dir(temp.path().join(name)).unwrap();
        std::fs::write(temp.path().join(name).join("test_wait.py"),
            "import unittest, time\nclass Wait(unittest.TestCase):\n def test_wait(self): time.sleep(3)\n").unwrap();
    }
    let manifest = temp.path().join("workspace.json");
    std::fs::write(&manifest, r#"{"projects":{"backend":"backend"}}"#).unwrap();
    let mut server = Server::start(&manifest);
    let started = server.call("harness_jobs", json!({"action":"start","verify":{"language":"python","action":"test","scope":"backend:/test_wait.py"}}));
    let task = started["data"]["taskId"].as_str().unwrap();
    std::fs::write(&manifest, r#"{"projects":{"frontend":"frontend"}}"#).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    let old = server.call("harness_jobs", json!({"action":"status","taskId":task}));
    assert_eq!(old["data"]["project"], "backend");
    server.call("harness_jobs", json!({"action":"cancel","taskId":task}));
    let response = server.request("tools/call", json!({"name":"harness_jobs","arguments":{"action":"start","verify":{"language":"python","action":"test","scope":"backend:/test_wait.py"}}}));
    assert_eq!(response["result"]["structuredContent"]["state"], "error");
    let new_job = server.call("harness_jobs", json!({"action":"start","verify":{"language":"python","action":"build","scope":"frontend:/test_wait.py"}}));
    assert_eq!(new_job["data"]["project"], "frontend");
    assert!(
        temp.path().join("frontend/.agents/skills").exists(),
        "registered projects initialize skills"
    );
}

#[test]
fn workspace_stdio_reuses_session_for_both_roots_and_edits_search_paths() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["backend", "frontend"] {
        std::fs::create_dir(temp.path().join(name)).unwrap();
        std::fs::write(
            temp.path().join(name).join("same.txt"),
            format!("needle-{name}\n"),
        )
        .unwrap();
    }
    let manifest = temp.path().join("workspace.json");
    std::fs::write(
        &manifest,
        json!({"projects":{
            "backend":{"root":"backend"}, "frontend":{"root":"frontend"}
        }})
        .to_string(),
    )
    .unwrap();
    let mut server = Server::start(&manifest);
    let bootstrap = server.call("harness_bootstrap", json!({}));
    let session = bootstrap["data"]["session"].as_str().unwrap();
    let limited = server.call(
        "harness_search",
        json!({
            "kind":"text", "query":"needle", "limit":1, "session":session
        }),
    );
    assert_eq!(limited["data"]["matches"].as_array().unwrap().len(), 1);
    let detail = server.call(
        "harness_detail",
        json!({"ref":limited["detail"]["ref"],"session":session}),
    );
    let full: Value = serde_json::from_str(detail["data"]["text"].as_str().unwrap()).unwrap();
    assert_eq!(full["matches"].as_array().unwrap().len(), 2, "{detail}");
    let search = server.call(
        "harness_search",
        json!({
            "kind":"text", "query":"needle", "contextLines":1, "session":session
        }),
    );
    let matches = search["data"]["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 2, "{search}");
    for name in ["backend", "frontend"] {
        let path = format!("{name}:/same.txt");
        let found = matches
            .iter()
            .find(|m| m["file"] == path)
            .expect("qualified search path");
        let read = server.call(
            "harness_read",
            json!({"path":found["file"],"start":1,"end":1,"session":session}),
        );
        assert_eq!(read["data"]["path"], path, "{read}");
        server.call(
            "harness_replace",
            json!({
                "path":path,
                "oldText":format!("needle-{name}"),"newText":format!("updated-{name}"),
                "allowUnvalidatedText":true,"session":session
            }),
        );
        assert_eq!(
            std::fs::read_to_string(temp.path().join(name).join("same.txt")).unwrap(),
            format!("updated-{name}\n")
        );
    }
}

#[test]
#[ignore = "requires Node and installed official Angular packages in HARNESS_ANGULAR_FIXTURE"]
fn angular_official_compiler_works_through_workspace_mcp_and_jobs() {
    let dependencies =
        std::env::var_os("HARNESS_ANGULAR_FIXTURE").expect("set HARNESS_ANGULAR_FIXTURE");
    let temp = tempfile::tempdir_in(dependencies).unwrap();
    let frontend = temp.path().join("frontend");
    let backend = temp.path().join("backend");
    std::fs::create_dir(&frontend).unwrap();
    std::fs::create_dir(&backend).unwrap();
    std::fs::write(frontend.join("angular.json"), json!({"projects":{"web":{"architect":{"build":{"options":{"tsConfig":"tsconfig.app.json"}}}}}}).to_string()).unwrap();
    std::fs::write(frontend.join("tsconfig.app.json"), json!({
        "compilerOptions":{"target":"ES2022","module":"ES2022","moduleResolution":"bundler","experimentalDecorators":true,"strict":true,"skipLibCheck":true,"types":[],"lib":["es2022","dom"]},
        "angularCompilerOptions":{"strictTemplates":true},"files":["app.ts"]
    }).to_string()).unwrap();
    std::fs::write(frontend.join("app.ts"), "import {Component} from '@angular/core'; @Component({selector:'test-app',standalone:true,templateUrl:'./app.html'}) export class App { title = 'ready'; }").unwrap();
    std::fs::write(frontend.join("app.html"), "{{title}}").unwrap();
    let manifest = temp.path().join("workspace.json");
    std::fs::write(
        &manifest,
        json!({"projects":{"backend":{"root":"backend"},"frontend":{"root":"frontend"}}})
            .to_string(),
    )
    .unwrap();
    let mut server = Server::start(&manifest);
    let catalog = server.request("tools/list", json!({}));
    let tool = catalog["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "harness_verify")
        .unwrap();
    assert!(
        tool["inputSchema"].to_string().contains("configuration"),
        "{tool}"
    );
    let bootstrap = server.call("harness_bootstrap", json!({}));
    let session = bootstrap["data"]["session"].as_str().unwrap();
    let valid = server.call(
        "harness_verify",
        json!({"language":"angular","scope":"frontend:/","project":"web","session":session}),
    );
    assert_eq!(
        valid["data"]["compiler"], "@angular/compiler-cli",
        "{valid}"
    );
    assert_eq!(valid["data"]["strictTemplates"], true);
    assert_eq!(valid["data"]["passed"], true);
    std::fs::write(frontend.join("app.html"), "{{missing}}").unwrap();
    let response = server.request("tools/call", json!({"name":"harness_verify","arguments":{"language":"angular","scope":"frontend:/","project":"web","session":session}}));
    let invalid = &response["result"]["structuredContent"];
    assert_eq!(invalid["state"], "error", "{invalid}");
    assert!(
        invalid["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "TS2339" && d["path"] == "frontend:/app.html"),
        "{invalid}"
    );
    let started = server.call("harness_jobs", json!({"action":"start","verify":{"language":"angular","scope":"frontend:/","project":"web"},"timeoutSeconds":45}));
    assert_eq!(started["data"]["project"], "frontend");
    let task = started["data"]["taskId"].as_str().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(50);
    loop {
        let status = server.call("harness_jobs", json!({"action":"status","taskId":task}));
        if status["data"]["status"] == "failed" {
            assert_eq!(
                status["data"]["result"]["compiler"], "@angular/compiler-cli",
                "{status}"
            );
            assert_eq!(status["data"]["result"]["passed"], false);
            assert!(
                status["data"]["result"]["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["code"] == "TS2339"),
                "{status}"
            );
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{status}");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!frontend.join("app.js").exists());
}
