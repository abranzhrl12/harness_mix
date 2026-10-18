#![cfg(windows)]
use agent_harness::adapters::index_store::{initialize, rebuild_workspace, sync_workspace_all};
use harness_domain::execution::OperationContext;
use rusqlite::Connection;
use std::fs;

fn count(root: &std::path::Path, query: &str) -> i64 {
    Connection::open(root.join(".codegraph/codegraph.db"))
        .unwrap()
        .query_row(query, [], |r| r.get(0))
        .unwrap()
}

#[test]
fn includes_gitignored_source_but_exclusion_and_safety_skips_win() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    fs::create_dir(root.path().join("Tools")).unwrap();
    fs::create_dir(root.path().join("node_modules")).unwrap();
    fs::write(root.path().join(".gitignore"), "Tools/\n").unwrap();
    for file in [
        "normal.rs",
        "Tools/yes.rs",
        "Tools/no.rs",
        "node_modules/hidden.rs",
    ] {
        fs::write(root.path().join(file), "pub fn sample() {}\n").unwrap();
    }
    fs::write(
        root.path().join("codegraph.json"),
        r#"{"include":["Tools/","node_modules/"],"exclude":["Tools/no.rs"]}"#,
    )
    .unwrap();
    let result = sync_workspace_all(root.path(), &context).unwrap();
    assert_eq!(result["selectedScopeApplied"], true, "{result}");
    assert_eq!(count(root.path(), "SELECT count(*) FROM files"), 2);
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM files WHERE path='Tools/yes.rs'"
        ),
        1
    );
}

#[test]
fn excluding_tracked_source_removes_graph_not_disk_and_removing_config_restores_it() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    fs::write(root.path().join("a.rs"), "pub fn keep() {}\n").unwrap();
    sync_workspace_all(root.path(), &context).unwrap();
    fs::write(
        root.path().join("codegraph.json"),
        r#"{"exclude":["a.rs"]}"#,
    )
    .unwrap();
    let result = sync_workspace_all(root.path(), &context).unwrap();
    assert_eq!(result["selectedScopeApplied"], true, "{result}");
    assert_eq!(count(root.path(), "SELECT count(*) FROM files"), 0);
    assert!(root.path().join("a.rs").is_file());
    fs::remove_file(root.path().join("codegraph.json")).unwrap();
    sync_workspace_all(root.path(), &context).unwrap();
    assert_eq!(count(root.path(), "SELECT count(*) FROM files"), 1);
}

#[test]
fn invalid_or_unimplemented_configuration_never_commits_sources() {
    for config in [
        r#"{"extensions":{}}"#,
        r#"{"includeIgnored":["packages/"]}"#,
        r#"{"exclude":["../escape"]}"#,
        r#"{"exclude":["!x"]}"#,
        r#"{"include":false}"#,
        "{",
    ] {
        let root = tempfile::tempdir().unwrap();
        let context = OperationContext::default();
        initialize(root.path(), &context).unwrap();
        fs::write(root.path().join("a.rs"), "fn sample() {}").unwrap();
        fs::write(root.path().join("codegraph.json"), config).unwrap();
        assert!(
            sync_workspace_all(root.path(), &context).is_err(),
            "{config}"
        );
        assert_eq!(count(root.path(), "SELECT count(*) FROM files"), 0);
    }
}

#[test]
fn rebuild_reextracts_unchanged_sources_without_deleting_index_or_metadata() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    fs::write(root.path().join("a.rs"), "pub fn sample() {}\n").unwrap();
    sync_workspace_all(root.path(), &context).unwrap();
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    db.execute("DELETE FROM nodes WHERE name='sample'", [])
        .unwrap();
    db.execute(
        "INSERT INTO project_metadata VALUES('user-marker','preserved',0)",
        [],
    )
    .unwrap();
    drop(db);
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE name='sample'"
        ),
        0
    );
    let result = rebuild_workspace(root.path(), &context).unwrap();
    assert_eq!(result["selectedScopeApplied"], true, "{result}");
    assert_eq!(result["changedFilesCommitted"], 1);
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE name='sample'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM project_metadata WHERE key='user-marker'"
        ),
        1
    );
}

#[test]
fn synchronized_explore_updates_changes_and_default_remains_read_only() {
    use harness_domain::contracts::Request;
    let root = tempfile::tempdir().unwrap();
    initialize(root.path(), &OperationContext::default()).unwrap();
    fs::write(
        root.path().join("lib.rs"),
        "pub fn fresh() {}\npub fn caller(){fresh();}\n",
    )
    .unwrap();
    let app = agent_harness::adapters::application(root.path()).unwrap();
    let request =
        Request::Explore(serde_json::from_value(serde_json::json!({"query":"fresh"})).unwrap());
    assert!(!request.is_mutating());
    app.execute(request);
    assert_eq!(count(root.path(), "SELECT count(*) FROM files"), 0);
    let request = Request::Explore(
        serde_json::from_value(serde_json::json!({"query":"fresh","syncIndex":true})).unwrap(),
    );
    assert!(request.is_mutating());
    let result = app.execute(request);
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["sync"]["changedFilesCommitted"], 1);
    assert_eq!(result["data"]["focus"]["status"], "resolved");
    assert!(result["data"]["text"].as_str().unwrap().contains("caller"));
    fs::write(
        root.path().join("lib.rs"),
        "pub fn fresh() {}\npub fn replacement(){fresh();}\n",
    )
    .unwrap();
    let result = app.execute(Request::Explore(
        serde_json::from_value(serde_json::json!({"query":"fresh","syncIndex":true})).unwrap(),
    ));
    assert_eq!(result["state"], "ok", "{result}");
    assert!(
        result["data"]["text"]
            .as_str()
            .unwrap()
            .contains("replacement")
    );
}

#[test]
fn synchronized_explore_preserves_partial_commits_and_validates_before_writing() {
    use harness_domain::contracts::Request;
    let root = tempfile::tempdir().unwrap();
    initialize(root.path(), &OperationContext::default()).unwrap();
    fs::write(root.path().join("a.rs"), "pub fn fresh() {}\n").unwrap();
    let app = agent_harness::adapters::application(root.path()).unwrap();
    let result = app.execute(Request::Explore(
        serde_json::from_value(serde_json::json!({"query":"fresh()","syncIndex":true})).unwrap(),
    ));
    assert_eq!(result["state"], "error");
    assert_eq!(count(root.path(), "SELECT count(*) FROM files"), 0);
    fs::write(root.path().join("z.ts"), "export type * from './absent';").unwrap();
    let result = app.execute(Request::Explore(
        serde_json::from_value(serde_json::json!({"query":"fresh","syncIndex":true})).unwrap(),
    ));
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(result["data"]["sync"]["changedFilesCommitted"], 1);
    assert_eq!(result["data"]["explorationPerformed"], false);
    assert!(result["data"]["graph"].is_null());
}

#[test]
fn synchronized_explore_accepts_multiple_identifier_selectors() {
    use harness_domain::contracts::Request;
    let root = tempfile::tempdir().unwrap();
    initialize(root.path(), &OperationContext::default()).unwrap();
    fs::write(
        root.path().join("lib.rs"),
        "pub fn fresh() {}\npub fn caller(){fresh();}\n",
    )
    .unwrap();
    let app = agent_harness::adapters::application(root.path()).unwrap();
    let result = app.execute(Request::Explore(
        serde_json::from_value(serde_json::json!({"query":"fresh caller","syncIndex":true}))
            .unwrap(),
    ));
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["sync"]["changedFilesCommitted"], 1);
    assert_eq!(count(root.path(), "SELECT count(*) FROM files"), 1);
    let text = result["data"]["text"].as_str().unwrap();
    assert!(
        text.contains("fresh") && text.contains("caller"),
        "{result}"
    );
}
