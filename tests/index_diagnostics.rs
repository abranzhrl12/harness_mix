#[path = "support/codegraph.rs"]
mod graph_fixture;

use agent_harness::adapters;
use harness_domain::contracts::{ExploreArgs, Request};
use serde_json::{Value, json};

fn explore(root: &std::path::Path) -> Value {
    let app = adapters::application(root).unwrap();
    let args: ExploreArgs = serde_json::from_value(json!({"query":"target"})).unwrap();
    app.execute(Request::Explore(args))
}

#[test]
fn unsupported_schema_reports_actual_version_and_exact_database_path() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    db.execute("UPDATE schema_versions SET version=999", [])
        .unwrap();
    let result = explore(root.path());
    assert_eq!(result["state"], "error", "{result}");
    let detail = &result["data"]["indexDiagnostic"];
    assert_eq!(detail["cause"], "unsupported_schema_version");
    assert_eq!(detail["actualSchemaVersion"], 999);
    assert_eq!(detail["stage"], "schema_validation");
    assert_eq!(
        detail["databasePath"],
        json!(
            root.path()
                .canonicalize()
                .unwrap()
                .join(".codegraph/codegraph.db")
        )
    );
}

#[test]
fn exclusive_sqlite_lock_is_reported_as_busy_without_raw_error_text() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    db.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let result = explore(root.path());
    assert_eq!(result["state"], "error", "{result}");
    let detail = &result["data"]["indexDiagnostic"];
    assert_eq!(detail["cause"], "database_busy", "{result}");
    assert_eq!(detail["sqliteCode"], "DatabaseBusy");
    assert!(detail["sqliteExtendedCode"].is_number());
    db.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn malformed_database_is_distinguished_from_schema_mismatch() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".codegraph")).unwrap();
    std::fs::write(root.path().join(".codegraph/codegraph.db"), b"not sqlite").unwrap();
    let result = explore(root.path());
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(result["data"]["indexDiagnostic"]["cause"], "not_a_database");
}

#[test]
fn missing_database_reports_path_and_does_not_create_an_index() {
    let root = tempfile::tempdir().unwrap();
    let result = explore(root.path());
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(
        result["data"]["indexDiagnostic"]["cause"],
        "database_missing"
    );
    assert!(!root.path().join(".codegraph").exists());
}

#[test]
#[cfg(windows)]
fn sync_handles_a_source_with_more_than_a_thousand_references() {
    use agent_harness::adapters::index_store;
    use harness_domain::execution::OperationContext;
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::with_timeout(std::time::Duration::from_secs(120));
    index_store::initialize(root.path(), &context).unwrap();
    let source = format!(
        "pub fn target() {{}}\npub fn caller() {{ {} }}\n",
        "target();".repeat(1200)
    );
    std::fs::write(root.path().join("lib.rs"), source).unwrap();
    let report = index_store::sync_workspace_all(root.path(), &context).unwrap();
    assert_eq!(report["selectedScopeApplied"], true, "{report}");
    assert_eq!(report["nativeFilesRemaining"], 0);
    assert!(
        report["files"][0]["referencesAttempted"].as_u64().unwrap() >= 1200,
        "{report}"
    );
}
