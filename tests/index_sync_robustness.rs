#![cfg(windows)]

use agent_harness::adapters::index_store::{sync_file, sync_file_with_tsconfig};
use harness_domain::execution::OperationContext;
use rusqlite::Connection;
use std::{fs, path::Path};

fn fixture(root: &Path) {
    fs::create_dir_all(root.join(".codegraph")).unwrap();
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0)",
        [],
    )
    .unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL").unwrap();
}

fn sync(root: &Path, path: &str) -> serde_json::Value {
    sync_file(root, path, &OperationContext::default()).unwrap()
}

fn calls_to(root: &Path, target: &str) -> i64 {
    Connection::open(root.join(".codegraph/codegraph.db"))
        .unwrap()
        .query_row(
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path=?1",
            [target],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn barrel_export_resolution_tracks_new_ambiguous_and_removed_contributors() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("left.ts"), "export function run() {}\n").unwrap();
    fs::write(root.path().join("right.ts"), "export function run() {}\n").unwrap();
    fs::write(root.path().join("barrel.ts"), "export * from './left';\n").unwrap();
    fs::write(
        root.path().join("app.ts"),
        "import {run} from './barrel'; export function caller() { run(); }\n",
    )
    .unwrap();
    sync(root.path(), "left.ts");
    sync(root.path(), "right.ts");
    sync(root.path(), "barrel.ts");
    sync(root.path(), "app.ts");
    assert_eq!(calls_to(root.path(), "left.ts"), 1);

    // A second distinct binding makes the barrel export ambiguous.
    fs::write(
        root.path().join("barrel.ts"),
        "export * from './left'; export * from './right';\n",
    )
    .unwrap();
    let result = sync(root.path(), "barrel.ts");
    assert_eq!(result["importReferencesRechecked"], 1, "{result}");
    assert_eq!(calls_to(root.path(), "left.ts"), 0);

    // Removing that contributor restores the original binding.
    fs::write(root.path().join("barrel.ts"), "export * from './left';\n").unwrap();
    sync(root.path(), "barrel.ts");
    assert_eq!(calls_to(root.path(), "left.ts"), 1);

    // Removing the remaining export also removes the call edge.
    fs::write(root.path().join("barrel.ts"), "export {};\n").unwrap();
    sync(root.path(), "barrel.ts");
    assert_eq!(calls_to(root.path(), "left.ts"), 0);
}

#[test]
fn tsconfig_path_mapping_changes_are_observed_on_the_next_publication() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("one.ts"), "export function run() {}\n").unwrap();
    fs::write(root.path().join("two.ts"), "export function run() {}\n").unwrap();
    fs::write(
        root.path().join("tsconfig.json"),
        r#"{"compilerOptions":{"paths":{"@lib":["missing.ts"]}}}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("app.ts"),
        "import {run} from '@lib'; export function caller() { run(); }\n",
    )
    .unwrap();
    sync(root.path(), "one.ts");
    sync(root.path(), "two.ts");
    sync_file_with_tsconfig(
        root.path(),
        "app.ts",
        "tsconfig.json",
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(
        calls_to(root.path(), "one.ts") + calls_to(root.path(), "two.ts"),
        0
    );

    fs::write(
        root.path().join("tsconfig.json"),
        r#"{"compilerOptions":{"paths":{"@lib":["one.ts"]}}}"#,
    )
    .unwrap();
    let result = sync_file_with_tsconfig(
        root.path(),
        "app.ts",
        "tsconfig.json",
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(result["committed"], true, "{result}");
    assert_eq!(calls_to(root.path(), "one.ts"), 1);

    fs::write(
        root.path().join("tsconfig.json"),
        r#"{"compilerOptions":{"paths":{"@lib":["two.ts"]}}}"#,
    )
    .unwrap();
    sync_file_with_tsconfig(
        root.path(),
        "app.ts",
        "tsconfig.json",
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(calls_to(root.path(), "one.ts"), 0);
    assert_eq!(calls_to(root.path(), "two.ts"), 1);
}
