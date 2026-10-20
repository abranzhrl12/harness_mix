use agent_harness::adapters::index_sync::plan_workspace;
use harness_domain::{execution::OperationContext, hash};
use rusqlite::params;
use serde_json::Value;
use std::{fs, process::Command};
#[path = "support/codegraph.rs"]
mod graph_fixture;

#[test]
fn razor_inventory_and_preview_include_views_and_respect_ignores() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    drop(db);
    fs::write(
        root.path().join("Index.cshtml"),
        "@functions { public void Run() {} }",
    )
    .unwrap();
    fs::write(root.path().join("Hidden.cshtml"), "@section Hidden {}").unwrap();
    fs::write(root.path().join(".gitignore"), "Hidden.cshtml\n").unwrap();
    let result = plan_workspace(root.path(), &OperationContext::default()).unwrap();
    assert_eq!(
        result["plan"]["added"],
        serde_json::json!(["Index.cshtml"]),
        "{result}"
    );
    assert!(
        result["extractionPreview"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "Index.cshtml"
                && file["status"] == "extracted_not_published")
    );
}

#[test]
fn native_inventory_drives_plan_without_writing_index_or_sources() {
    let dir = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(dir.path());
    for (path, content) in [
        ("old.cs", "class Old {}"),
        ("gone.rs", "fn gone() {}"),
        ("bad.rs", "broken"),
    ] {
        fs::write(dir.path().join(path), content).unwrap();
        db.execute(
            "INSERT INTO files VALUES(?1,?2,NULL)",
            params![path, hash(content.as_bytes())],
        )
        .unwrap();
    }
    drop(db);
    fs::write(dir.path().join("old.cs"), "class Changed {}").unwrap();
    fs::remove_file(dir.path().join("gone.rs")).unwrap();
    fs::write(dir.path().join("new.rs"), "fn new() {}\r\n").unwrap();
    fs::write(dir.path().join("new.cs"), "class NotDiscoveredYet {}").unwrap();
    fs::write(dir.path().join("bad.rs"), [0xff]).unwrap();
    let before = fs::read(dir.path().join(".codegraph/codegraph.db")).unwrap();
    let result = plan_workspace(dir.path(), &OperationContext::default()).unwrap();
    assert_eq!(
        result["plan"]["added"],
        serde_json::json!(["new.cs", "new.rs"])
    );
    assert_eq!(result["extractionPreview"]["published"], false);
    assert_eq!(result["extractionPreview"]["relationsResolved"], false);
    assert_eq!(
        result["extractionPreview"]["files"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(
        result["extractionPreview"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|file| file["status"] == "extracted_not_published")
    );
    assert_eq!(result["plan"]["modified"], serde_json::json!(["old.cs"]));
    assert_eq!(result["plan"]["removed"], serde_json::json!(["gone.rs"]));
    assert_eq!(result["plan"]["unchecked"], serde_json::json!(["bad.rs"]));
    assert_eq!(result["plan"]["unchanged"], serde_json::json!(["lib.rs"]));
    assert_eq!(result["coverage"]["fullProviderScope"], false);
    assert_eq!(result["synchronized"], false);
    assert_eq!(
        before,
        fs::read(dir.path().join(".codegraph/codegraph.db")).unwrap()
    );
}

#[test]
fn discovery_honors_local_gitignores_and_does_not_remove_ignored_tracked_files() {
    let dir = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(dir.path());
    fs::write(dir.path().join(".gitignore"), "ignored.rs\nlib.rs\n").unwrap();
    fs::write(dir.path().join("ignored.rs"), "fn ignored() {}").unwrap();
    for folder in ["target", "node_modules", "vendor", ".git"] {
        fs::create_dir_all(dir.path().join(folder)).unwrap();
        fs::write(dir.path().join(folder).join("ignored.rs"), "fn hidden() {}").unwrap();
    }
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/.gitignore"), "local.rs\n").unwrap();
    fs::write(dir.path().join("src/local.rs"), "fn local() {}").unwrap();
    fs::write(dir.path().join("src/visible.rs"), "fn visible() {}").unwrap();
    drop(db);
    let result = plan_workspace(dir.path(), &OperationContext::default()).unwrap();
    assert_eq!(
        result["plan"]["added"],
        serde_json::json!(["src/visible.rs"])
    );
    assert_eq!(result["plan"]["unchanged"], serde_json::json!(["lib.rs"]));
    assert_eq!(result["plan"]["removed"], serde_json::json!([]));
}

#[test]
fn missing_index_cancellation_unsupported_schema_and_restricted_paths_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    assert!(plan_workspace(dir.path(), &OperationContext::default()).is_err());
    assert!(!dir.path().join(".codegraph").exists());
    let db = graph_fixture::fixture(dir.path());
    let cancelled = OperationContext::default();
    cancelled.cancel();
    assert_eq!(
        plan_workspace(dir.path(), &cancelled).unwrap_err().0,
        "CANCELLED"
    );
    db.execute(
        "INSERT INTO files VALUES('.env/private.rs',?1,NULL)",
        [hash(b"secret")],
    )
    .unwrap();
    let restricted = plan_workspace(dir.path(), &OperationContext::default()).unwrap();
    assert_eq!(restricted["inventory"]["restrictedIndexedFiles"], 1);
    assert!(
        !serde_json::to_string(&restricted)
            .unwrap()
            .contains("private.rs")
    );
    assert!(!serde_json::to_string(&restricted).unwrap().contains(".env"));
    assert_eq!(restricted["plan"]["removed"], serde_json::json!([]));
    db.execute("DELETE FROM files WHERE path='.env/private.rs'", [])
        .unwrap();
    db.execute("INSERT INTO schema_versions VALUES(999)", [])
        .unwrap();
    assert_eq!(
        plan_workspace(dir.path(), &OperationContext::default())
            .unwrap_err()
            .0,
        "INDEX"
    );
}

#[test]
fn oversized_tracked_sources_are_unknown_not_clean_or_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(dir.path());
    let bytes = vec![b'x'; 1024 * 1024 + 1];
    fs::write(dir.path().join("large.rs"), &bytes).unwrap();
    db.execute(
        "INSERT INTO files VALUES('large.rs',?1,NULL)",
        [hash(
            format!("codegraph:oversize:{}", bytes.len()).as_bytes(),
        )],
    )
    .unwrap();
    drop(db);
    let result = plan_workspace(dir.path(), &OperationContext::default()).unwrap();
    assert_eq!(result["plan"]["unchecked"], serde_json::json!(["large.rs"]));
    assert!(result["inventory"]["reservedSourceBytes"].as_u64().unwrap() < 1024 * 1024);
}

#[test]
fn same_size_and_mtime_changes_are_detected_by_content_not_stat_shortcuts() {
    let dir = tempfile::tempdir().unwrap();
    drop(graph_fixture::fixture(dir.path()));
    let path = dir.path().join("lib.rs");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let original = fs::read_to_string(&path).unwrap();
    let changed = original.replacen("target", "TARGET", 1);
    assert_eq!(original.len(), changed.len());
    fs::write(&path, changed).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let result = plan_workspace(dir.path(), &OperationContext::default()).unwrap();
    assert_eq!(result["plan"]["modified"], serde_json::json!(["lib.rs"]));
    assert_eq!(result["plan"]["unchanged"], serde_json::json!([]));
}

#[test]
fn cli_plan_works_without_node_or_git_and_returns_json() {
    let dir = tempfile::tempdir().unwrap();
    drop(graph_fixture::fixture(dir.path()));
    let output = Command::new(env!("CARGO_BIN_EXE_agent-harness"))
        .env("PATH", "")
        .args(["--root"])
        .arg(dir.path())
        .arg("--sync-plan")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["mode"], "plan_only");
    assert_eq!(result["writesPerformed"], false);
    assert_eq!(result["plan"]["unchanged"], serde_json::json!(["lib.rs"]));
}

#[cfg(windows)]
#[test]
fn linked_source_directories_are_never_followed_or_treated_as_deletions() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(dir.path());
    fs::write(outside.path().join("hidden.rs"), "fn hidden() {}").unwrap();
    // Junctions don't require Developer Mode. This command only creates the
    // explicitly named fixture junction; it never deletes or moves anything.
    let output = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(dir.path().join("linked"))
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    db.execute(
        "INSERT INTO files VALUES('linked/hidden.rs',?1,NULL)",
        [hash(b"fn hidden() {}")],
    )
    .unwrap();
    drop(db);
    let result = plan_workspace(dir.path(), &OperationContext::default()).unwrap();
    assert_eq!(
        result["plan"]["unchecked"],
        serde_json::json!(["linked/hidden.rs"])
    );
    assert_eq!(result["plan"]["removed"], serde_json::json!([]));
    assert_eq!(result["plan"]["added"], serde_json::json!([]));
}
