#![cfg(windows)]
use agent_harness::adapters::{self, NativeApplication};
use harness_domain::{
    contracts::{MoveArgs, Request},
    execution::OperationContext,
    tool_contract::ToolProfile,
};
use serde_json::{Value, json};
use std::fs;

fn args(from: &str, to: &str) -> MoveArgs {
    MoveArgs {
        from: from.into(),
        to: to.into(),
        session: None,
    }
}
fn move_file(app: &NativeApplication, from: &str, to: &str) -> Value {
    app.execute(Request::Move(args(from, to)))
}

#[test]
fn moves_without_reading_or_modifying_contents_and_rejects_retries() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("dest")).unwrap();
    let bytes = b"\0\xff\r\nunchanged";
    fs::write(root.path().join("source.bin"), bytes).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = move_file(&app, "source.bin", "dest/renamed.bin");
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["changed"], true);
    assert_eq!(result["data"]["verified"], true);
    assert_eq!(result["data"]["index"]["status"], "not_present");
    assert_eq!(result["data"]["importsUpdated"], false);
    assert!(!root.path().join("source.bin").exists());
    assert_eq!(
        fs::read(root.path().join("dest/renamed.bin")).unwrap(),
        bytes
    );
    assert_eq!(
        move_file(&app, "source.bin", "dest/renamed.bin")["state"],
        "error"
    );
    assert!(!root.path().join(".codegraph").exists());
}

#[test]
fn rejects_existing_same_case_only_protected_missing_parent_and_escaping_targets() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source.txt"), "original").unwrap();
    fs::write(root.path().join("existing.txt"), "keep").unwrap();
    fs::create_dir(root.path().join("directory")).unwrap();
    let app = adapters::application(root.path()).unwrap();
    for to in [
        "existing.txt",
        "source.txt",
        "SOURCE.txt",
        "directory",
        "missing/file.txt",
        "../outside.txt",
        "C:/outside.txt",
        ".env",
        "AGENTS.md",
        ".agents/state",
        ".codegraph/codegraph.db",
        ".git/config",
        "directory/AGENTS.md",
    ] {
        let result = move_file(&app, "source.txt", to);
        assert_eq!(result["state"], "error", "{to}: {result}");
        assert_eq!(
            fs::read_to_string(root.path().join("source.txt")).unwrap(),
            "original"
        );
    }
    for from in ["directory", "missing.txt", "../outside.txt"] {
        assert_eq!(move_file(&app, from, "new.txt")["state"], "error");
    }
    fs::write(root.path().join("AGENTS.md"), "instructions").unwrap();
    assert_eq!(
        move_file(&app, "AGENTS.md", "new.txt")["diagnostics"][0]["code"],
        "PATH"
    );
    fs::hard_link(
        root.path().join("source.txt"),
        root.path().join("linked.txt"),
    )
    .unwrap();
    assert_eq!(
        move_file(&app, "linked.txt", "new.txt")["diagnostics"][0]["code"],
        "PATH"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("existing.txt")).unwrap(),
        "keep"
    );
    assert!(!root.path().join("new.txt").exists());
}

#[test]
fn cancellation_and_investigation_profile_prevent_moves() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source.txt"), "original").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let context = OperationContext::default();
    context.cancel();
    let result = app.execute_with_context(Request::Move(args("source.txt", "new.txt")), context);
    assert_eq!(result["state"], "cancelled");
    let app = app.with_tool_profile(ToolProfile::Investigate);
    let denied = move_file(&app, "source.txt", "new.txt");
    assert_eq!(denied["diagnostics"][0]["code"], "TOOL_PROFILE");
    assert!(root.path().join("source.txt").exists());
    assert!(!root.path().join("new.txt").exists());
    assert!(ToolProfile::Edit.allows("harness_move"));
    assert!(Request::Move(args("a", "b")).is_mutating());
    assert!(
        serde_json::from_value::<MoveArgs>(json!({"from":"a","to":"b","overwrite":true})).is_err()
    );
}

#[test]
fn junctions_cannot_redirect_either_side_of_a_move() {
    use std::{os::windows::process::CommandExt, process::Command};
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source.txt"), "inside").unwrap();
    fs::write(outside.path().join("outside.txt"), "outside").unwrap();
    let junction = root.path().join("junction");
    let status = Command::new("cmd")
        .creation_flags(0x0800_0000)
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(status.status.success());
    let app = adapters::application(root.path()).unwrap();
    for (from, to) in [
        ("source.txt", "junction/new.txt"),
        ("junction/outside.txt", "new.txt"),
    ] {
        let result = move_file(&app, from, to);
        assert_eq!(result["diagnostics"][0]["code"], "PATH", "{result}");
    }
    assert_eq!(
        fs::read_to_string(root.path().join("source.txt")).unwrap(),
        "inside"
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("outside.txt")).unwrap(),
        "outside"
    );
    assert!(!outside.path().join("new.txt").exists());
    assert!(!root.path().join("new.txt").exists());
    fs::remove_dir(junction).unwrap();
}

#[test]
fn updates_index_for_both_paths_and_removes_stale_import_edges() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("dest")).unwrap();
    fs::write(
        root.path().join("source.ts"),
        "export function moved() { return 1; }",
    )
    .unwrap();
    fs::write(
        root.path().join("caller.ts"),
        "import { moved } from './source';\nexport function caller() { return moved(); }",
    )
    .unwrap();
    let context = OperationContext::default();
    adapters::index_store::initialize(root.path(), &context).unwrap();
    let synced = adapters::index_store::sync_workspace_all(root.path(), &context).unwrap();
    assert_eq!(synced["selectedScopeApplied"], true, "{synced}");
    let app = adapters::application(root.path()).unwrap();
    let result = move_file(&app, "source.ts", "dest/renamed.ts");
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["index"]["status"], "updated", "{result}");
    let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(
        count("SELECT count(*) FROM files WHERE path='source.ts'"),
        0
    );
    assert_eq!(
        count("SELECT count(*) FROM files WHERE path='dest/renamed.ts'"),
        1
    );
    assert_eq!(
        count("SELECT count(*) FROM nodes WHERE file_path='source.ts'"),
        0
    );
    assert!(count("SELECT count(*) FROM nodes WHERE file_path='dest/renamed.ts'") > 0);
    assert_eq!(
        count(
            "SELECT count(*) FROM edges e JOIN nodes src ON src.id=e.source JOIN nodes dst ON dst.id=e.target WHERE src.file_path='caller.ts' AND dst.file_path='dest/renamed.ts'"
        ),
        0
    );
    assert!(
        fs::read_to_string(root.path().join("caller.ts"))
            .unwrap()
            .contains("'./source'")
    );
}

#[test]
fn index_failure_reports_completed_move_instead_of_hiding_it() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".codegraph")).unwrap();
    fs::write(
        root.path().join(".codegraph/codegraph.db"),
        "invalid database",
    )
    .unwrap();
    fs::write(root.path().join("source.rs"), "fn original() {}").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = move_file(&app, "source.rs", "renamed.rs");
    assert_eq!(result["data"]["changed"], true, "{result}");
    assert_eq!(result["data"]["commitStatus"], "committed");
    assert_eq!(result["data"]["verified"], true);
    assert_eq!(result["data"]["index"]["status"], "needs_sync");
    assert_eq!(
        result["diagnostics"][0]["requiredAction"],
        "move_completed_refresh_index_only"
    );
    assert!(!root.path().join("source.rs").exists());
    assert_eq!(
        fs::read_to_string(root.path().join("renamed.rs")).unwrap(),
        "fn original() {}"
    );
}

#[test]
fn session_records_move_and_removes_obsolete_edit_observations() {
    use harness_domain::{
        contracts::{BootstrapArgs, CheckpointArgs, ReplaceArgs, TasksArgs},
        hash,
    };
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source.txt"), "old").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        session: None,
        include_rules: None,
    }));
    let session = boot["data"]["sessionId"].as_str().unwrap();
    let edit: ReplaceArgs = serde_json::from_value(json!({"session":session,"path":"source.txt","hash":hash(b"old"),"oldText":"old","newText":"new"})).unwrap();
    assert_eq!(app.execute(Request::Replace(edit))["state"], "ok");
    let mut move_args = args("source.txt", "target.txt");
    move_args.session = Some(session.into());
    let result = app.execute(Request::Move(move_args));
    assert_eq!(result["state"], "ok", "{result}");
    assert!(result["data"].get("sessionWarning").is_none(), "{result}");
    let checkpoint: CheckpointArgs = serde_json::from_value(json!({"session":session})).unwrap();
    let state = app.execute(Request::Checkpoint(checkpoint));
    assert!(
        state["data"]["observedByHarness"]["modifiedFiles"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{state}"
    );
    let tasks: TasksArgs =
        serde_json::from_value(json!({"session":session,"action":"timeline"})).unwrap();
    let timeline = app.execute(Request::Tasks(tasks));
    assert!(
        timeline["data"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["type"] == "file_moved"
                && event["from"] == "source.txt"
                && event["to"] == "target.txt"),
        "{timeline}"
    );
}
