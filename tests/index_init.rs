#![cfg(windows)]
use agent_harness::adapters::index_store::{initialize, sync_workspace, sync_workspace_all};
use harness_domain::execution::OperationContext;
use rusqlite::Connection;
use std::{fs, process::Command};

#[test]
fn fresh_index_accepts_native_sync_and_fts_without_provider() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    let report = initialize(root.path(), &context).unwrap();
    assert_eq!(report["filesIndexed"], 0);
    assert_eq!(report["synchronized"], false);
    fs::write(root.path().join("lib.ts"), "export function work() {}").unwrap();
    fs::write(
        root.path().join("app.ts"),
        "import {work} from './lib'; export function entry(){work();}",
    )
    .unwrap();
    let report = sync_workspace(root.path(), &context).unwrap();
    assert_eq!(report["selectedScopeApplied"], true, "{report}");
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    let edges: i64 = db.query_row("SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.name='entry' AND t.name='work'", [], |r| r.get(0)).unwrap();
    assert_eq!(edges, 1);
    let fts: i64 = db
        .query_row(
            "SELECT count(*) FROM nodes_fts WHERE nodes_fts MATCH 'work'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(fts > 0);
    let journal: String = db
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(journal, "wal");
}

#[test]
fn existing_directory_or_database_is_never_overwritten() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".codegraph")).unwrap();
    let file = root.path().join(".codegraph/codegraph.db");
    fs::write(&file, b"existing user index").unwrap();
    assert_eq!(
        initialize(root.path(), &OperationContext::default())
            .unwrap_err()
            .0,
        "INIT_EXISTS"
    );
    assert_eq!(fs::read(file).unwrap(), b"existing user index");
}

#[test]
fn duplicate_initialize_preserves_published_sources() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    fs::write(root.path().join("main.rs"), "fn preserved() {}").unwrap();
    sync_workspace(root.path(), &context).unwrap();
    assert_eq!(
        initialize(root.path(), &context).unwrap_err().0,
        "INIT_EXISTS"
    );
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM nodes WHERE name='preserved'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn cli_bootstraps_and_syncs_with_empty_path() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("main.rs"), "fn native_only() {}").unwrap();
    let exe = env!("CARGO_BIN_EXE_agent-harness");
    for option in ["--init-index", "--sync"] {
        let output = Command::new(exe)
            .args(["--root", root.path().to_str().unwrap(), option])
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(report["durationMs"].is_number());
        let progress = String::from_utf8_lossy(&output.stderr);
        assert!(progress.contains("complete"), "{progress}");
        if option == "--sync" {
            assert!(progress.contains("[inventory]"), "{progress}");
            assert!(progress.contains("[indexing]"), "{progress}");
            assert!(progress.contains("1/1"), "{progress}");
            assert!(report["files"][0]["durationMs"].is_number());
        }
    }
}

#[test]
fn linked_index_directory_does_not_modify_its_target() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    if std::os::windows::fs::symlink_dir(outside.path(), root.path().join(".codegraph")).is_err() {
        // Windows may not grant symlink creation to the test process.
        return;
    }
    assert!(initialize(root.path(), &OperationContext::default()).is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn cancelled_initialize_does_not_create_a_namespace() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    context.cancel();
    assert_eq!(
        initialize(root.path(), &context).unwrap_err().0,
        "CANCELLED"
    );
    assert!(!root.path().join(".codegraph").exists());
}

#[test]
fn progress_counts_confirmed_files_and_cancellation_preserves_prior_commit() {
    let root = tempfile::tempdir().unwrap();
    let owner = OperationContext::default();
    initialize(root.path(), &owner).unwrap();
    for name in ["a.rs", "b.rs"] {
        fs::write(root.path().join(name), "fn indexed() {}").unwrap();
    }
    let observed_root = root.path().to_owned();
    let cancel = owner.clone();
    let context = owner.with_progress(move |update| {
        if update.phase == "indexing" && update.completed == 1 {
            assert_eq!(update.total, Some(2));
            // The count must only advance once another connection sees the commit.
            let db = Connection::open(observed_root.join(".codegraph/codegraph.db")).unwrap();
            let count: i64 = db
                .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 1);
            cancel.cancel();
        }
    });
    let report = sync_workspace_all(root.path(), &context).unwrap();
    assert_eq!(report["selectedScopeApplied"], false, "{report}");
    assert_eq!(report["filesProcessed"], 1);
    assert_eq!(report["changedFilesCommitted"], 1);
    assert_eq!(report["failure"]["code"], "CANCELLED");
    assert_eq!(report["nativeFilesRemaining"], 1);
}

#[test]
fn a_busy_index_stops_the_batch_after_one_bounded_wait() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    for name in ["a.rs", "b.rs", "c.rs"] {
        fs::write(root.path().join(name), "fn waits() {}").unwrap();
    }
    let blocker = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let report = sync_workspace_all(root.path(), &context).unwrap();
    blocker.execute_batch("ROLLBACK").unwrap();
    assert_eq!(report["failure"]["code"], "SYNC_INDEX_BUSY", "{report}");
    assert_eq!(report["files"].as_array().unwrap().len(), 1);
    assert_eq!(report["failedFiles"], 1);
    assert_eq!(report["filesProcessed"], 0);
    assert_eq!(report["nativeFilesRemaining"], 3);
}

#[test]
fn full_sync_processes_more_than_32_files_with_bounded_report() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    for index in 0..35 {
        fs::write(
            root.path().join(format!("file{index:02}.rs")),
            format!("fn symbol_{index}() {{}} "),
        )
        .unwrap();
    }
    let report = sync_workspace_all(root.path(), &context).unwrap();
    assert_eq!(report["selectedScopeApplied"], true, "{report}");
    assert_eq!(report["changedFilesCommitted"], 35);
    assert_eq!(report["filesProcessed"], 35);
    assert_eq!(report["nativeFilesRemaining"], 0);
    assert_eq!(report["fileReportsOmitted"], 3);
    assert_eq!(report["files"].as_array().unwrap().len(), 32);
    // The planner now opens/pins the writer to inspect resolver versions, so
    // the very first publication reuses that connection too.
    assert_eq!(report["files"][0]["writerConnectionReused"], true);
    assert!(
        report["files"].as_array().unwrap()[1..]
            .iter()
            .all(|file| file["writerConnectionReused"] == true)
    );
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 35);
}

#[test]
fn full_sync_failure_keeps_prior_commits_in_aggregate_report() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    fs::write(root.path().join("a.rs"), "fn preserved() {}").unwrap();
    fs::write(root.path().join("z.ts"), "export type * from './missing';").unwrap();
    let report = sync_workspace_all(root.path(), &context).unwrap();
    assert_eq!(report["selectedScopeApplied"], false, "{report}");
    assert_eq!(report["changedFilesCommitted"], 1);
    assert_eq!(report["filesProcessed"], 1);
    assert_eq!(report["failure"]["path"], "z.ts");
    assert_eq!(report["failure"]["commitState"], "not_committed");
}

#[test]
fn a_deferred_source_does_not_stop_the_rest_of_the_workspace() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    fs::write(root.path().join("a.rs"), "fn preserved() {}").unwrap();
    fs::write(root.path().join("m.ts"), "export type * from './missing';").unwrap();
    fs::write(root.path().join("z.rs"), "fn afterwards() {}").unwrap();
    let report = sync_workspace_all(root.path(), &context).unwrap();
    assert_eq!(report["failure"]["path"], "m.ts", "{report}");
    assert_eq!(report["changedFilesCommitted"], 2, "{report}");
    assert_eq!(report["filesProcessed"], 2, "{report}");
    assert_eq!(report["failedFiles"], 1, "{report}");
}

#[test]
fn fresh_index_can_be_explored_without_provider() {
    use harness_domain::contracts::Request;
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    fs::write(
        root.path().join("lib.rs"),
        "pub fn work() {}\npub fn entry() { work(); }\n",
    )
    .unwrap();
    let report = sync_workspace_all(root.path(), &context).unwrap();
    assert_eq!(report["selectedScopeApplied"], true, "{report}");
    let app = agent_harness::adapters::application(root.path()).unwrap();
    let args = serde_json::from_value(serde_json::json!({"query":"work","symbol":"work"})).unwrap();
    let result = app.execute(Request::Explore(args));
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["focus"]["status"], "resolved", "{result}");
    assert!(
        result["data"]["text"].as_str().unwrap().contains("entry"),
        "{result}"
    );
}
