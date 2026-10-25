//! Production RA writer regression: lifecycle and post-publication rollback.
//! These tests intentionally exercise the real Windows writer and SQLite transaction.

use super::{initialize, sync_file, sync_file_with};
use harness_domain::execution::OperationContext;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

fn write(root: &Path, path: &str, contents: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, contents).unwrap();
}

fn publish(root: &Path, path: &str) {
    let result = sync_file(root, path, &OperationContext::default()).unwrap();
    assert_eq!(result["committed"], true, "{path}: {result}");
}

fn fixture(root: &Path, caller: &str) {
    initialize(root, &OperationContext::default()).unwrap();
    for (path, source) in [
        ("src/lib.rs", "pub mod area;\n"),
        ("src/area/mod.rs", "pub mod api;\npub mod caller;\n"),
        (
            "src/area/api.rs",
            "/// Indexed API\npub fn alpha() {}\npub fn beta() {}\npub fn gamma() {}\n",
        ),
    ] {
        write(root, path, source);
        publish(root, path);
    }
    write(root, "src/area/caller.rs", caller);
    publish(root, "src/area/caller.rs");
}

fn caller_edges(
    root: &Path,
) -> Vec<(
    String,
    String,
    String,
    String,
    Option<i64>,
    Option<i64>,
    Option<String>,
)> {
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    let mut statement = db.prepare(
        "SELECT e.kind,s.file_path,s.name,t.name,e.line,e.col,json_extract(e.metadata,'$.resolvedBy') FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE s.file_path='src/area/caller.rs' AND e.kind='calls' ORDER BY t.name,e.line,e.col",
    ).unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn caller_pending(root: &Path) -> Vec<(String, String, String, i64, i64)> {
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    let mut statement = db.prepare(
        "SELECT r.reference_name,r.reference_kind,r.status,r.line,r.col FROM unresolved_refs r JOIN nodes n ON n.id=r.from_node_id WHERE n.file_path='src/area/caller.rs' ORDER BY r.reference_name,r.line,r.col",
    ).unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn caller_hash(root: &Path) -> String {
    Connection::open(root.join(".codegraph/codegraph.db"))
        .unwrap()
        .query_row(
            "SELECT content_hash FROM files WHERE path='src/area/caller.rs'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn emit(root: &Path, checkpoint: &str) {
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    let target_hash: String = db
        .query_row(
            "SELECT content_hash FROM files WHERE path='src/area/api.rs'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    let state: Value = json!({
        "checkpoint": checkpoint,
        "callerHash": caller_hash(root),
        "targetHash": target_hash,
        "edges": caller_edges(root),
        "pending": caller_pending(root)
    });
    println!(
        "RA_WRITER_REPLAY:{}",
        serde_json::to_string(&state).unwrap()
    );
}

#[test]
fn rust_writer_replay_tracks_target_and_module_lifecycle() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), "pub fn invoke() { super::api::alpha(); }\n");
    assert_eq!(caller_edges(root.path()).len(), 1);
    assert!(caller_pending(root.path()).is_empty());
    emit(root.path(), "initial-alpha");

    write(
        root.path(),
        "src/area/api.rs",
        "/// Indexed API\npub fn beta() {}\npub fn gamma() {}\n",
    );
    publish(root.path(), "src/area/api.rs");
    assert!(caller_edges(root.path()).is_empty());
    assert!(
        caller_pending(root.path())
            .iter()
            .any(|r| r.0 == "super::api::alpha")
    );
    emit(root.path(), "target-alpha-removed");

    write(
        root.path(),
        "src/area/caller.rs",
        "pub fn invoke() { super::api::beta(); }\n",
    );
    publish(root.path(), "src/area/caller.rs");
    assert_eq!(
        caller_edges(root.path())
            .iter()
            .map(|e| e.3.as_str())
            .collect::<Vec<_>>(),
        ["beta"]
    );
    assert!(caller_pending(root.path()).is_empty());
    emit(root.path(), "caller-edited-to-beta");

    fs::remove_file(root.path().join("src/area/api.rs")).unwrap();
    publish(root.path(), "src/area/api.rs");
    assert!(caller_edges(root.path()).is_empty());
    assert!(
        caller_pending(root.path())
            .iter()
            .any(|r| r.0 == "super::api::beta")
    );
    emit(root.path(), "target-deleted");

    write(
        root.path(),
        "src/area/api.rs",
        "/// Indexed API\npub fn beta() {}\npub fn gamma() {}\n",
    );
    publish(root.path(), "src/area/api.rs");
    assert_eq!(
        caller_edges(root.path())
            .iter()
            .map(|e| e.3.as_str())
            .collect::<Vec<_>>(),
        ["beta"]
    );
    assert!(caller_pending(root.path()).is_empty());
    emit(root.path(), "target-readded");

    write(
        root.path(),
        "src/area/mod.rs",
        "#[path=\"alternate.rs\"] pub mod api;\npub mod caller;\n",
    );
    publish(root.path(), "src/area/mod.rs");
    assert!(caller_edges(root.path()).is_empty());
    assert!(
        caller_pending(root.path())
            .iter()
            .any(|r| r.0 == "super::api::beta")
    );
    emit(root.path(), "module-path-changed");

    write(
        root.path(),
        "src/area/mod.rs",
        "pub mod api;\npub mod caller;\n",
    );
    publish(root.path(), "src/area/mod.rs");
    assert_eq!(
        caller_edges(root.path())
            .iter()
            .map(|e| e.3.as_str())
            .collect::<Vec<_>>(),
        ["beta"]
    );
    emit(root.path(), "module-path-restored");

    write(root.path(), "src/area/api/mod.rs", "pub fn beta() {}\n");
    publish(root.path(), "src/area/api/mod.rs");
    assert!(caller_edges(root.path()).is_empty());
    assert!(
        caller_pending(root.path())
            .iter()
            .any(|r| r.0 == "super::api::beta")
    );
    emit(root.path(), "module-target-ambiguous");

    fs::remove_file(root.path().join("src/area/api/mod.rs")).unwrap();
    publish(root.path(), "src/area/api/mod.rs");
    assert_eq!(
        caller_edges(root.path())
            .iter()
            .map(|e| e.3.as_str())
            .collect::<Vec<_>>(),
        ["beta"]
    );
    emit(root.path(), "module-ambiguity-removed");
}

#[test]
fn rust_type_reference_keeps_homonymous_import_unresolved() {
    let root = tempfile::tempdir().unwrap();
    initialize(root.path(), &OperationContext::default()).unwrap();
    for (path, source) in [
        ("src/lib.rs", "pub mod area;\npub mod experiments;\n"),
        (
            "src/area/mod.rs",
            "pub mod caller;\npub mod project;\n",
        ),
        (
            "experiments/json-performance/src/main.rs",
            "pub struct Project;\n",
        ),
        ("src/area/project.rs", "pub struct Project;\n"),
        (
            "src/area/caller.rs",
            "use super::project::Project;\npub fn invoke(_: Project) {}\n",
        ),
    ] {
        write(root.path(), path, source);
        publish(root.path(), path);
    }

    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    let false_edge: i64 = db
        .query_row(
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE s.file_path='src/area/caller.rs' AND t.file_path='experiments/json-performance/src/main.rs' AND t.name='Project' AND e.kind='references'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(false_edge, 0, "a same-named type is not an import proof");

    let unresolved: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM unresolved_refs WHERE file_path='src/area/caller.rs' AND reference_name='Project' AND reference_kind='references' AND status='pending')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(unresolved, "unsupported type-import resolution stays explicit");
}

#[test]
fn rust_writer_replay_rolls_back_resolved_edges_on_cancel_and_timeout() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), "pub fn invoke() { super::api::alpha(); }\n");
    let prior_hash = caller_hash(root.path());
    let prior_edges = caller_edges(root.path());
    emit(root.path(), "before-cancel");

    write(
        root.path(),
        "src/area/caller.rs",
        "pub fn invoke() { super::api::gamma(); }\n",
    );
    let cancelled = OperationContext::default();
    let cancel_checkpoint = AtomicBool::new(false);
    let result = sync_file_with(root.path(), "src/area/caller.rs", &cancelled, || {
        cancel_checkpoint.store(true, Ordering::Release);
        cancelled.cancel();
    });
    assert_eq!(result.unwrap_err().0, "CANCELLED");
    assert!(cancel_checkpoint.load(Ordering::Acquire));
    assert_eq!(caller_hash(root.path()), prior_hash);
    assert_eq!(caller_edges(root.path()), prior_edges);
    assert!(caller_pending(root.path()).is_empty());
    emit(root.path(), "after-cancel-rollback");

    write(
        root.path(),
        "src/area/caller.rs",
        "pub fn invoke() { super::api::beta(); }\n",
    );
    let timed = OperationContext::with_timeout(Duration::from_secs(10));
    let timeout_checkpoint = AtomicBool::new(false);
    let result = sync_file_with(root.path(), "src/area/caller.rs", &timed, || {
        timeout_checkpoint.store(true, Ordering::Release);
        std::thread::sleep(Duration::from_millis(10_100));
    });
    assert_eq!(result.unwrap_err().0, "TIMEOUT");
    assert!(timeout_checkpoint.load(Ordering::Acquire));
    assert_eq!(caller_hash(root.path()), prior_hash);
    assert_eq!(caller_edges(root.path()), prior_edges);
    assert!(caller_pending(root.path()).is_empty());
    emit(root.path(), "after-timeout-rollback");
}
