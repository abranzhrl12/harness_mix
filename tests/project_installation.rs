#![cfg(windows)]
use serde_json::Value;
use std::{fs, process::Command};

#[test]
fn one_command_installs_indexes_and_uninstalls_the_project() {
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
    let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    let calls: i64 = db
        .query_row("SELECT count(*) FROM edges WHERE kind='calls'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(calls, 1);
    drop(db);
    let again = run("init");
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    let report: Value = serde_json::from_slice(&again.stdout).unwrap();
    assert_eq!(report["index"]["changedFilesCommitted"], 0);
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
