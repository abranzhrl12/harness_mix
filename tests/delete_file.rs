#![cfg(windows)]
use agent_harness::adapters::{self, NativeApplication};
use harness_domain::{
    contracts::{DeleteArgs, Request},
    execution::OperationContext,
    hash,
    tool_contract::ToolProfile,
};
use serde_json::{Value, json};
use std::fs;

fn args(path: &str) -> DeleteArgs {
    DeleteArgs {
        path: path.into(),
        expected_hash: None,
        session: None,
    }
}
fn delete(app: &NativeApplication, path: &str) -> Value {
    app.execute(Request::Delete(args(path)))
}

#[test]
fn deletes_binary_and_does_not_create_index_or_backups() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("old.bin"), b"\0\xff\r\n").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = delete(&app, "old.bin");
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["changed"], true);
    assert_eq!(result["data"]["verified"], true);
    assert_eq!(result["data"]["permanent"], true);
    assert_eq!(result["data"]["index"]["status"], "not_present");
    assert!(!root.path().join("old.bin").exists());
    assert!(!root.path().join(".codegraph").exists());
    assert_eq!(delete(&app, "old.bin")["state"], "error");
}

#[test]
fn stale_or_invalid_hash_never_deletes_and_matching_hash_does() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file.txt"), "current").unwrap();
    let app = adapters::application(root.path()).unwrap();
    for (expected, code) in [(hash(b"previous"), "STALE"), ("invalid".into(), "ARGUMENT")] {
        let mut a = args("file.txt");
        a.expected_hash = Some(expected);
        let result = app.execute(Request::Delete(a));
        assert_eq!(result["diagnostics"][0]["code"], code, "{result}");
        assert_eq!(
            fs::read_to_string(root.path().join("file.txt")).unwrap(),
            "current"
        );
    }
    let mut a = args("file.txt");
    a.expected_hash = Some(hash(b"current").to_uppercase());
    assert_eq!(app.execute(Request::Delete(a))["state"], "ok");
    assert!(!root.path().join("file.txt").exists());
}

#[test]
fn refuses_directories_protected_paths_aliases_and_hard_links() {
    let root = tempfile::tempdir().unwrap();
    for folder in ["folder", ".git", ".agents", ".codegraph"] {
        fs::create_dir(root.path().join(folder)).unwrap();
        fs::write(root.path().join(folder).join("keep.txt"), "keep").unwrap();
    }
    for name in ["AGENTS.md", ".env", "keep.txt"] {
        fs::write(root.path().join(name), "keep").unwrap();
    }
    let app = adapters::application(root.path()).unwrap();
    for path in [
        ".",
        "folder",
        "folder/AGENTS.md",
        "AGENTS.md",
        ".env",
        ".git/keep.txt",
        ".agents/keep.txt",
        ".codegraph/keep.txt",
        "../outside.txt",
        "C:/outside.txt",
        "keep.txt.",
        "*.txt",
    ] {
        let result = delete(&app, path);
        assert_eq!(result["state"], "error", "{path}: {result}");
    }
    fs::hard_link(root.path().join("keep.txt"), root.path().join("linked.txt")).unwrap();
    assert_eq!(delete(&app, "linked.txt")["diagnostics"][0]["code"], "PATH");
    assert_eq!(
        fs::read_to_string(root.path().join("keep.txt")).unwrap(),
        "keep"
    );
    assert!(root.path().join("folder/keep.txt").exists());
    assert!(root.path().join("AGENTS.md").exists());
}

#[test]
fn cancellation_profile_and_busy_file_prevent_deletion() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("keep.txt");
    fs::write(&path, "keep").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let context = OperationContext::default();
    context.cancel();
    assert_eq!(
        app.execute_with_context(Request::Delete(args("keep.txt")), context)["state"],
        "cancelled"
    );
    use std::os::windows::fs::OpenOptionsExt;
    let reader = fs::File::options()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    assert_eq!(
        delete(&app, "keep.txt")["diagnostics"][0]["code"],
        "EDIT_BUSY"
    );
    drop(reader);
    let app = app.with_tool_profile(ToolProfile::Investigate);
    assert_eq!(
        delete(&app, "keep.txt")["diagnostics"][0]["code"],
        "TOOL_PROFILE"
    );
    assert!(path.exists());
    assert!(ToolProfile::Edit.allows("harness_delete"));
    assert!(Request::Delete(args("keep.txt")).is_mutating());
    assert!(
        serde_json::from_value::<DeleteArgs>(json!({"path":"keep.txt","recursive":true})).is_err()
    );
}

#[test]
fn junction_cannot_redirect_delete_outside_workspace() {
    use std::{os::windows::process::CommandExt, process::Command};
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("keep.txt"), "keep").unwrap();
    let junction = root.path().join("junction");
    let output = Command::new("cmd")
        .creation_flags(0x0800_0000)
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let app = adapters::application(root.path()).unwrap();
    assert_eq!(
        delete(&app, "junction/keep.txt")["diagnostics"][0]["code"],
        "PATH"
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("keep.txt")).unwrap(),
        "keep"
    );
    fs::remove_dir(junction).unwrap();
}

#[test]
fn index_failure_reports_committed_deletion() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".codegraph")).unwrap();
    fs::write(root.path().join(".codegraph/codegraph.db"), "invalid").unwrap();
    fs::write(root.path().join("obsolete.ts"), "export function old() {}").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = delete(&app, "obsolete.ts");
    assert_eq!(result["data"]["changed"], true, "{result}");
    assert_eq!(result["data"]["verified"], true);
    assert_eq!(result["data"]["index"]["status"], "needs_sync");
    assert_eq!(
        result["diagnostics"][0]["requiredAction"],
        "delete_completed_refresh_index_only"
    );
    assert!(!root.path().join("obsolete.ts").exists());
}

#[test]
fn deletion_removes_indexed_file_nodes_and_incoming_edges() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("old.ts"),
        "export function old() { return 1; }",
    )
    .unwrap();
    fs::write(
        root.path().join("caller.ts"),
        "import { old } from './old'; export function caller() { return old(); }",
    )
    .unwrap();
    let context = OperationContext::default();
    adapters::index_store::initialize(root.path(), &context).unwrap();
    adapters::index_store::sync_workspace_all(root.path(), &context).unwrap();
    let app = adapters::application(root.path()).unwrap();
    // Windows accepts case variants; the index must remove the actual stored spelling.
    let result = delete(&app, "OLD.ts");
    assert_eq!(result["data"]["index"]["status"], "updated", "{result}");
    let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    for sql in [
        "SELECT count(*) FROM files WHERE path='old.ts'",
        "SELECT count(*) FROM nodes WHERE file_path='old.ts'",
        "SELECT count(*) FROM edges e LEFT JOIN nodes n ON n.id=e.target WHERE n.id IS NULL",
    ] {
        assert_eq!(
            db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap(),
            0,
            "{sql}"
        );
    }
    assert!(root.path().join("caller.ts").exists());
}

#[test]
fn session_records_deletion_and_removes_obsolete_edit_observations() {
    use harness_domain::contracts::{BootstrapArgs, CheckpointArgs, ReplaceArgs, TasksArgs};
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file.txt"), "old").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        compact: true,
        session: None,
        include_rules: None,
    }));
    let session = boot["data"]["session"].as_str().unwrap();
    let edit: ReplaceArgs = serde_json::from_value(
        json!({"session":session,"path":"file.txt","oldText":"old","newText":"new"}),
    )
    .unwrap();
    assert_eq!(app.execute(Request::Replace(edit))["state"], "ok");
    let mut a = args("file.txt");
    a.session = Some(session.into());
    let result = app.execute(Request::Delete(a));
    assert_eq!(result["state"], "ok", "{result}");
    assert!(result["data"].get("sessionWarning").is_none());
    let a: CheckpointArgs = serde_json::from_value(json!({"session":session})).unwrap();
    let state = app.execute(Request::Checkpoint(a));
    assert!(
        state["data"]["observedByHarness"]["modifiedFiles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let a: TasksArgs =
        serde_json::from_value(json!({"session":session,"action":"timeline"})).unwrap();
    let timeline = app.execute(Request::Tasks(a));
    assert!(
        timeline["data"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["type"] == "file_deleted" && event["path"] == "file.txt")
    );
}
