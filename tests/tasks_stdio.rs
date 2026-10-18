use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct Server {
    child: Child,
    messages: mpsc::Receiver<Value>,
    events: Vec<Value>,
}
impl Server {
    fn start(root: &std::path::Path) -> Self {
        Self::start_with_profile(root, "full")
    }
    fn start_with_profile(root: &std::path::Path, profile: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_agent-harness"))
            .arg("--root")
            .arg(root)
            .env("HARNESS_TOOL_PROFILE", profile)
            .env_remove("CARGO_TARGET_DIR")
            .env("CARGO_BUILD_JOBS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(serde_json::from_str(&line).unwrap()).is_err() {
                    break;
                }
            }
        });
        let mut server = Self {
            child,
            messages,
            events: Vec::new(),
        };
        let init = server.request(1, "initialize", json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"jobs-test","version":"1"}}));
        assert!(
            init["result"]["capabilities"]["logging"].is_object(),
            "{init}"
        );
        server.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        server
    }
    fn send(&mut self, value: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{value}").unwrap();
        stdin.flush().unwrap();
    }
    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        loop {
            let message = self
                .messages
                .recv_timeout(Duration::from_secs(20))
                .expect("MCP response");
            if message["id"] == id {
                return message;
            }
            self.events.push(message);
        }
    }
    fn jobs(&mut self, id: u64, arguments: Value) -> Value {
        let response = self.request(
            id,
            "tools/call",
            json!({"name":"harness_jobs","arguments":arguments}),
        );
        let envelope = &response["result"]["structuredContent"];
        assert_eq!(envelope["state"], "ok", "{response}");
        envelope["data"].clone()
    }
    fn task_event(&mut self, task_id: &str, wanted: &[&str]) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let matches = |message: &Value| {
                message["method"] == "notifications/message"
                    && message["params"]["data"]["taskId"] == task_id
                    && wanted.contains(
                        &message["params"]["data"]["data"]["status"]
                            .as_str()
                            .unwrap_or(""),
                    )
            };
            if let Some(index) = self.events.iter().position(matches) {
                let message = self.events.remove(index);
                assert_eq!(message["params"]["logger"], "harness.tasks");
                return message["params"]["data"]["data"].clone();
            }
            let message = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("task push notification");
            if matches(&message) {
                assert_eq!(message["params"]["logger"], "harness.tasks");
                return message["params"]["data"]["data"].clone();
            }
            self.events.push(message);
        }
    }
    fn close(&mut self) {
        drop(self.child.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "server did not close cleanly: {status}");
                return;
            }
            assert!(
                Instant::now() < deadline,
                "server did not close after stdin EOF"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn replay(&mut self, task_id: Option<&str>, mut after: u64) -> Vec<Value> {
        let mut events = Vec::new();
        loop {
            let page = self.jobs(
                100,
                json!({"action":"events","taskId":task_id,"after":after}),
            );
            events.extend(page["events"].as_array().unwrap().iter().cloned());
            if page["hasMore"] != true {
                return events;
            }
            let cursor = page["cursor"].as_u64().unwrap();
            assert!(cursor > after, "event pagination did not advance: {page}");
            after = cursor;
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn observe_filters_failed_job_output_and_advances_cursor_over_mcp() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), "pub fn broken() { let x = ; }\n");
    let mut server = Server::start(root.path());
    let accepted = server.jobs(2, json!({"action":"start","verify":{"language":"rust","action":"build"}}));
    let task_id = accepted["taskId"].as_str().unwrap();
    server.task_event(task_id, &["failed"]);
    let response = server.request(3, "tools/call", json!({"name":"harness_observe","arguments":{
        "job_id":task_id,"filter":"(?i)error","lines":2,"before":1,"after":2,"stream":"stderr"
    }}));
    let envelope = &response["result"]["structuredContent"];
    assert_eq!(envelope["state"], "ok", "{response}");
    let data = &envelope["data"];
    assert!(data["matchedLines"].as_u64().unwrap() > 0, "{data}");
    assert!(data["lines"].as_array().unwrap().iter().all(|line| line["stream"] == "stderr"));
    let next = server.request(4, "tools/call", json!({"name":"harness_observe","arguments":{
        "taskId":task_id,"cursor":data["cursor"]
    }}));
    assert_eq!(next["result"]["structuredContent"]["data"]["lines"], json!([]));
    let invalid = server.request(5, "tools/call", json!({"name":"harness_observe","arguments":{
        "taskId":task_id,"filter":"["
    }}));
    assert_eq!(invalid["result"]["structuredContent"]["diagnostics"][0]["code"], "ARGUMENT");
    server.close();
}

fn fixture(root: &std::path::Path, source: &str) {
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='durable-jobs-fixture'\nversion='0.1.0'\nedition='2024'\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(root.join("src/lib.rs"), source).unwrap();
}

#[test]
fn completed_job_result_and_event_cursor_survive_graceful_restart() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), "#[test] fn works() { assert_eq!(2+2,4); }\n");
    let mut server = Server::start(root.path());
    let accepted = server.jobs(
        2,
        json!({"action":"start","verify":{"language":"rust","action":"test"}}),
    );
    let task_id = accepted["taskId"].as_str().unwrap();
    server.task_event(task_id, &["succeeded", "failed"]);
    let completed = server.jobs(3, json!({"action":"status","taskId":task_id}));
    assert_eq!(completed["status"], "succeeded", "{completed}");
    assert!(completed["result"].is_object());
    let legacy = server.request(
        30,
        "tools/call",
        json!({"name":"harness_process_status","arguments":{"runId":task_id,"list":"recent"}}),
    );
    let legacy = &legacy["result"]["structuredContent"];
    assert_eq!(legacy["state"], "ok", "{legacy}");
    assert_eq!(legacy["data"]["run"]["state"], "succeeded");
    assert!(
        legacy["data"]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["taskId"] == task_id)
    );
    let report = server.request(
        31,
        "tools/call",
        json!({"name":"harness_doctor","arguments":{}}),
    );
    let report = &report["result"]["structuredContent"]["data"];
    assert_eq!(report["mcpRegistry"]["status"], "verified", "{report}");
    assert_eq!(report["allToolsVerified"], false);
    let storage = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["id"] == "background-jobs")
        .unwrap();
    assert_eq!(storage["status"], "ok", "{storage}");
    assert_eq!(storage["details"]["readChecked"], true);
    let before = server.replay(Some(task_id), 0);
    let cursor = before.last().unwrap()["cursor"].as_u64().unwrap();
    server.close();
    drop(server);

    // Restricted readers must never acquire the jobs writer lock.
    let mut investigate = Server::start_with_profile(root.path(), "investigate");
    let mut edit = Server::start_with_profile(root.path(), "edit");
    for restricted in [&mut investigate, &mut edit] {
        let catalog = restricted.request(2, "tools/list", json!({}));
        assert!(
            !catalog["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "harness_jobs")
        );
    }
    let mut restored = Server::start(root.path());
    let status = restored.jobs(2, json!({"action":"status","taskId":task_id}));
    assert_eq!(status, completed);
    let legacy = restored.request(
        30,
        "tools/call",
        json!({"name":"harness_process_status","arguments":{"runId":task_id}}),
    );
    assert_eq!(
        legacy["result"]["structuredContent"]["data"]["run"]["state"], "succeeded",
        "{legacy}"
    );
    let health = restored.jobs(3, json!({"action":"list"}));
    assert!(health["persistenceError"].is_null(), "{health}");
    let replay = restored.replay(Some(task_id), 0);
    assert_eq!(replay, before);
    let after = restored.jobs(4, json!({"action":"events","after":cursor}));
    assert!(after["events"].as_array().unwrap().is_empty(), "{after}");
    let next = restored.jobs(
        5,
        json!({"action":"start","verify":{"language":"rust","action":"test"}}),
    );
    let next_id = next["taskId"].as_str().unwrap();
    restored.task_event(next_id, &["succeeded", "failed"]);
    let newer = restored.replay(None, cursor);
    assert!(!newer.is_empty());
    assert!(
        newer
            .iter()
            .all(|event| event["cursor"].as_u64().unwrap() > cursor)
    );
    restored.close();
    investigate.close();
    edit.close();
}

#[test]
fn crash_recovers_active_job_as_interrupted_without_repeating_it() {
    let root = tempfile::tempdir().unwrap();
    fixture(
        root.path(),
        r#"#[test] fn blocked() {
        use std::io::Write;
        let marker = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("attempts.txt");
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(marker).unwrap();
        writeln!(file, "executed").unwrap();
        file.sync_all().unwrap();
        std::thread::sleep(std::time::Duration::from_secs(30));
    }"#,
    );
    let mut server = Server::start(root.path());
    let accepted = server.jobs(
        2,
        json!({"action":"start","verify":{"language":"rust","action":"test"},"maxAttempts":3}),
    );
    let task_id = accepted["taskId"].as_str().unwrap();
    server.task_event(task_id, &["running"]);
    let marker = root.path().join("attempts.txt");
    let deadline = Instant::now() + Duration::from_secs(60);
    while std::fs::read_to_string(&marker).unwrap_or_default() != "executed\n" {
        assert!(Instant::now() < deadline, "fixture test did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    server.child.kill().unwrap();
    server.child.wait().unwrap();
    drop(server);

    let mut restored = Server::start(root.path());
    let recovered = restored.jobs(2, json!({"action":"status","taskId":task_id}));
    assert_eq!(recovered["status"], "interrupted", "{recovered}");
    assert_eq!(recovered["attempt"], 1);
    assert_eq!(recovered["result"]["error"], "TASK_INTERRUPTED");
    let events = restored.replay(Some(task_id), 0);
    assert!(
        events
            .iter()
            .any(|event| event["data"]["status"] == "interrupted"),
        "{events:?}"
    );
    let tasks = restored.jobs(4, json!({"action":"list"}));
    assert!(tasks.to_string().contains("interrupted"), "{tasks}");
    restored.close();
    assert_eq!(
        std::fs::read_to_string(marker).unwrap(),
        "executed\n",
        "recovery must not rerun code"
    );
}

#[test]
fn background_job_returns_early_and_pushes_completion_without_polling() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("Cargo.toml"), "[package]\nname='background-stdio-fixture'\nversion='0.1.0'\nedition='2024'\n[workspace]\n").unwrap();
    std::fs::write(
        root.path().join("src/lib.rs"),
        "#[test] fn slow() { std::thread::sleep(std::time::Duration::from_secs(2)); }\n",
    )
    .unwrap();
    let mut server = Server::start(root.path());
    let accepted = Instant::now();
    let started = server.jobs(
        2,
        json!({"action":"start","verify":{"language":"rust","action":"test"}}),
    );
    assert!(
        accepted.elapsed() < Duration::from_secs(10) && started["status"] == "queued",
        "start waited for process completion: {started}"
    );
    let task_id = started["taskId"].as_str().expect("taskId").to_owned();
    let responsive = Instant::now();
    let tools = server.request(3, "tools/list", json!({}));
    assert!(tools["result"]["tools"].is_array());
    assert!(responsive.elapsed() < Duration::from_secs(10));
    let terminal = server.task_event(&task_id, &["succeeded", "failed", "cancelled", "timed_out"]);
    assert_eq!(terminal["status"], "succeeded", "{terminal}");
    let status = server.jobs(4, json!({"action":"status","taskId":task_id}));
    assert!(status.to_string().contains("succeeded"), "{status}");
    let events = server.jobs(5, json!({"action":"events","taskId":task_id,"after":0}));
    assert!(events.to_string().contains(&task_id), "{events}");
    let deadline_job = server.jobs(6, json!({"action":"start","verify":{"language":"rust","action":"test"},"timeoutSeconds":1,"maxAttempts":3}));
    let deadline_id = deadline_job["taskId"].as_str().unwrap();
    let timed_out = server.task_event(
        deadline_id,
        &["succeeded", "failed", "cancelled", "timed_out"],
    );
    assert_eq!(timed_out["status"], "timed_out", "{timed_out}");
    assert_eq!(
        timed_out["attempt"], 1,
        "a deadline must not trigger retries: {timed_out}"
    );

    let cancellation_job = server.jobs(
        7,
        json!({"action":"start","verify":{"language":"rust","action":"test"},"maxAttempts":3}),
    );
    let cancel_id = cancellation_job["taskId"].as_str().unwrap();
    server.task_event(cancel_id, &["running"]);
    server.jobs(8, json!({"action":"cancel","taskId":cancel_id}));
    let cancelled = server.task_event(
        cancel_id,
        &["succeeded", "failed", "cancelled", "timed_out"],
    );
    assert_eq!(cancelled["status"], "cancelled", "{cancelled}");
    assert_eq!(cancelled["attempt"], 1);

    std::fs::write(
        root.path().join("src/lib.rs"),
        "#[test] fn fails() { panic!(\"expected fixture failure\"); }\n",
    )
    .unwrap();
    let failure_job = server.jobs(
        9,
        json!({"action":"start","verify":{"language":"rust","action":"test"},"maxAttempts":3}),
    );
    let failed = server.task_event(
        failure_job["taskId"].as_str().unwrap(),
        &["succeeded", "failed", "cancelled", "timed_out"],
    );
    assert_eq!(failed["status"], "failed", "{failed}");
    assert_eq!(
        failed["attempt"], 1,
        "a failed verification must not trigger retries: {failed}"
    );

    let level = server.request(10, "logging/setLevel", json!({"level":"error"}));
    assert!(level.get("error").is_none(), "{level}");
}
