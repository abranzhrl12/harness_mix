use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc::{self, Receiver}},
    time::{Duration, Instant},
};

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-harness"));
    command
        .arg("--root")
        .arg(root)
        .env("HARNESS_INDEX_WATCH", "0");
    for key in [
        "HARNESS_PG_CONNECTION_STRING",
        "HARNESS_SQL_CONNECTION_STRING",
        "DATABASE_URL",
    ] {
        command.env_remove(key);
    }
    command
}
struct Mcp {
    child: Child,
    lines: Receiver<Result<String, String>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    seq: u64,
    session: String,
}
impl Mcp {
    fn new(root: &Path) -> Self {
        let mut child = command(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr_reader = child.stderr.take().unwrap();
        let (line_sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = line_sender.send(Err("MCP stdout closed".into()));
                        return;
                    }
                    Ok(_) => {
                        if line_sender.send(Ok(line)).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = line_sender.send(Err(format!("MCP stdout read failed: {error}")));
                        return;
                    }
                }
            }
        });
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let stderr_capture = stderr.clone();
        std::thread::spawn(move || {
            let mut reader = stderr_reader;
            let mut chunk = [0; 4096];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => return,
                    Ok(length) => {
                        let mut captured = stderr_capture.lock().unwrap();
                        captured.extend_from_slice(&chunk[..length]);
                        if captured.len() > 64 * 1024 {
                            let excess = captured.len() - 64 * 1024;
                            captured.drain(..excess);
                        }
                    }
                }
            }
        });
        let mut mcp = Self {
            child,
            lines,
            stderr,
            seq: 0,
            session: String::new(),
        };
        mcp.rpc("initialize", json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"config-test","version":"1"}}));
        writeln!(
            mcp.child.stdin.as_mut().unwrap(),
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .unwrap();
        let boot = mcp.tool("harness_bootstrap", json!({"includeRules":false}));
        mcp.session = boot["data"]["session"].as_str().unwrap().into();
        mcp
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.seq += 1;
        let deadline = Instant::now() + Duration::from_secs(20);
        let input = self.child.stdin.as_mut().unwrap();
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","id":self.seq,"method":method,"params":params})
        )
        .unwrap();
        input.flush().unwrap();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = match self.lines.recv_timeout(remaining) {
                Ok(Ok(line)) => line,
                Ok(Err(error)) => panic!("{error}; stderr: {}", self.stderr_text()),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("MCP output reader stopped; stderr: {}", self.stderr_text())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("MCP RPC {method} timed out after 20 seconds; stderr: {}", self.stderr_text())
                }
            };
            let result: Value = serde_json::from_str(&line).unwrap();
            if result["id"] == self.seq {
                assert!(result.get("error").is_none(), "{result}; stderr: {}", self.stderr_text());
                return result["result"].clone();
            }
        }
    }
    fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr.lock().unwrap()).into_owned()
    }
    fn tool(&mut self, name: &str, mut arguments: Value) -> Value {
        if !self.session.is_empty() {
            arguments["session"] = json!(self.session);
        }
        let response = self.rpc("tools/call", json!({"name":name,"arguments":arguments}));
        serde_json::from_str(response["content"][0]["text"].as_str().unwrap()).unwrap()
    }
    fn status(&mut self) -> Value {
        self.tool("harness_sql", json!({"action":"status","connect":false}))
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn env_only_selects_postgres_then_sqlserver_without_a_json_file() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join(".env"),
        "HARNESS_SQL_CONNECTION_STRING=postgresql://user:private-sentinel@localhost/test\n",
    )
    .unwrap();
    let mut mcp = Mcp::new(root.path());
    let status = mcp.status();
    assert_eq!(status["data"]["engine"], "postgresql", "{status}");
    assert_eq!(status["data"]["tlsMode"], "disable");
    assert_eq!(status["data"]["configured"], true);
    assert!(!status.to_string().contains("private-sentinel"));
    let protected = mcp.tool("harness_read", json!({"path":".env","start":1,"end":1}));
    assert_eq!(protected["diagnostics"][0]["code"], "PATH");
    assert!(!protected.to_string().contains("private-sentinel"));
    std::fs::write(root.path().join(".env"), "HARNESS_SQL_CONNECTION_STRING=Server=localhost;Database=test;User ID=user;Password=private-sentinel\n").unwrap();
    let status = mcp.status();
    assert_eq!(status["data"]["engine"], "sqlserver", "{status}");
    assert_eq!(status["data"]["configured"], true);
    assert!(!status.to_string().contains("private-sentinel"));
    assert!(!root.path().join(".agents/harness.config.json").exists());
}

#[test]
fn init_config_is_explicit_secret_free_and_never_overwrites() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join(".env"),
        "DATABASE_URL=postgresql://user:private-sentinel@localhost/test\n",
    )
    .unwrap();
    let result = command(root.path()).arg("--init-config").output().unwrap();
    assert!(result.status.success());
    let file = root.path().join(".agents/harness.config.json");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("private-sentinel"));
    let config: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(config["postgresql"]["connection"]["name"], "DATABASE_URL");
    let result = command(root.path()).arg("--init-config").output().unwrap();
    assert!(!result.status.success());
    assert_eq!(std::fs::read_to_string(file).unwrap(), text);
}

#[test]
fn explicit_disabled_configuration_overrides_env_detection() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".agents")).unwrap();
    std::fs::write(
        root.path().join(".env"),
        "DATABASE_URL=postgresql://user@localhost/test\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join(".agents/harness.config.json"),
        "{\"database\":{\"autoDetect\":false}}",
    )
    .unwrap();
    let mut mcp = Mcp::new(root.path());
    assert_eq!(mcp.status()["data"]["configured"], false);
}
