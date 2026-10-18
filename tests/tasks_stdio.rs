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
        let mut child = Command::new(env!("CARGO_BIN_EXE_agent-harness"))
            .arg("--root")
            .arg(root)
            .env("HARNESS_TOOL_PROFILE", "full")
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
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
