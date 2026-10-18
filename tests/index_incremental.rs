#![cfg(windows)]
use agent_harness::adapters::index_store::{initialize, sync_changed_paths, sync_file};
use harness_domain::execution::OperationContext;
use std::{collections::BTreeSet, fs};

#[test]
fn unchanged_event_skips_extraction_and_partial_batch_reports_confirmed_commits() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    fs::write(root.path().join("a.rs"), "pub fn first() {}\n").unwrap();
    let first = sync_file(root.path(), "a.rs", &context).unwrap();
    assert_eq!(first["extractionPerformed"], true);
    let duplicate = sync_file(root.path(), "a.rs", &context).unwrap();
    assert_eq!(duplicate["changed"], false);
    assert_eq!(duplicate["extractionPerformed"], false);
    fs::write(root.path().join("a.rs"), "pub fn changed() {}\n").unwrap();
    fs::write(root.path().join("z.rs"), "fn broken( {").unwrap();
    let result = sync_changed_paths(
        root.path(),
        &BTreeSet::from(["a.rs".into(), "z.rs".into()]),
        &context,
    )
    .unwrap();
    assert_eq!(result["selectedScopeApplied"], false);
    assert_eq!(result["changedFilesCommitted"], 1);
    assert_eq!(result["filesProcessed"], 1);
    assert_eq!(result["workspaceScanPerformed"], false);
    let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM nodes WHERE name='changed'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM files WHERE path='z.rs'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn targeted_discovery_respects_nested_ignores_excludes_and_explicit_includes() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::create_dir(root.path().join("blocked")).unwrap();
    fs::write(root.path().join("blocked/.gitignore"), "!resurrected.rs\n").unwrap();
    fs::create_dir(root.path().join(".git")).unwrap();
    fs::create_dir(root.path().join(".git/info")).unwrap();
    fs::write(root.path().join(".git/info/exclude"), "src/git.rs\n").unwrap();
    fs::write(root.path().join(".gitignore"), "src/root.rs\nblocked/\n").unwrap();
    fs::write(root.path().join("src/.gitignore"), "local.rs\n").unwrap();
    let names = [
        "blocked/resurrected.rs",
        "src/root.rs",
        "src/local.rs",
        "src/git.rs",
        "src/keep.rs",
        "src/excluded.rs",
    ];
    for name in names {
        fs::write(root.path().join(name), "pub fn sample() {}\n").unwrap();
    }
    fs::write(
        root.path().join("codegraph.json"),
        r#"{"include":["src/root.rs"],"exclude":["src/excluded.rs"]}"#,
    )
    .unwrap();
    let result = sync_changed_paths(
        root.path(),
        &names.into_iter().map(String::from).collect(),
        &context,
    )
    .unwrap();
    assert_eq!(result["selectedScopeApplied"], true, "{result}");
    assert_eq!(result["changedFilesCommitted"], 2, "{result}");
    let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM files", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}
