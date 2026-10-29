#![cfg(windows)]
use serde_json::Value;
use std::{fs, process::Command};

#[test]
fn init_installs_without_index_and_explicit_index_honors_scope() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("a.rs"),
        "pub fn first() {}\npub fn caller() { first(); }\n",
    )
    .unwrap();
    let run = |action: &str| {
        Command::new(env!("CARGO_BIN_EXE_agent-harness"))
            .current_dir(root.path())
            .arg(action)
            .output()
            .unwrap()
    };
    let first = run("init");
    assert!(
        first.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&first.stderr),
        String::from_utf8_lossy(&first.stdout)
    );
    let report: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["setupComplete"], true);
    assert!(
        root.path()
            .join(".agents/skills/harness-read/SKILL.md")
            .is_file()
    );
    assert!(root.path().join(".agents/mcp_config.json").is_file());
    assert!(root.path().join(".codex/config.toml").is_file());
    assert!(!root.path().join(".codegraph").exists());
    assert!(root.path().join(".harnessignore").is_file());
    let original_config = fs::read(root.path().join("codegraph.json")).unwrap();
    let original_ignore = fs::read(root.path().join(".harnessignore")).unwrap();
    fs::write(root.path().join("excluded.rs"), "pub fn excluded() {}\n").unwrap();
    fs::write(root.path().join("ignored.rs"), "pub fn ignored() {}\n").unwrap();
    fs::write(
        root.path().join("codegraph.json"),
        r#"{"exclude":["excluded.rs"]}"#,
    )
    .unwrap();
    fs::write(root.path().join(".harnessignore"), "ignored.rs\n").unwrap();
    let indexed = run("index");
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    let excluded: i64 = db
        .query_row(
            "SELECT count(*) FROM files WHERE path='excluded.rs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(excluded, 0);
    let ignored: i64 = db
        .query_row(
            "SELECT count(*) FROM files WHERE path='ignored.rs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ignored, 0);
    let calls: i64 = db
        .query_row("SELECT count(*) FROM edges WHERE kind='calls'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(calls, 1);
    drop(db);
    fs::write(root.path().join("later.rs"), "pub fn later() {}\n").unwrap();
    let again_index = run("index");
    assert!(
        again_index.status.success(),
        "{}",
        String::from_utf8_lossy(&again_index.stderr)
    );
    let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    let later: i64 = db
        .query_row(
            "SELECT count(*) FROM files WHERE path='later.rs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(later, 1);
    drop(db);
    let again = run("init");
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    let report: Value = serde_json::from_slice(&again.stdout).unwrap();
    assert_eq!(report["setupComplete"], true);
    fs::write(root.path().join("codegraph.json"), original_config).unwrap();
    fs::write(root.path().join(".harnessignore"), original_ignore).unwrap();
    let removed = run("uninstall");
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(!root.path().join(".codegraph").exists());
    assert!(!root.path().join(".agents").exists());
    assert!(!root.path().join(".codex").exists());
    assert!(root.path().join("a.rs").exists());
    assert!(run("uninstall").status.success());
}

#[test]
fn init_does_not_take_ownership_of_an_existing_index() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.rs"), "pub fn first() {}\n").unwrap();
    agent_harness::adapters::index_store::initialize(root.path(), &Default::default()).unwrap();
    let run = |action: &str| {
        Command::new(env!("CARGO_BIN_EXE_agent-harness"))
            .current_dir(root.path())
            .arg(action)
            .output()
            .unwrap()
    };
    let installed = run("init");
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    assert!(run("uninstall").status.success());
    assert!(root.path().join(".codegraph/codegraph.db").is_file());
}
