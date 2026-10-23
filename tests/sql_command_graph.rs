#![cfg(windows)]

use agent_harness::adapters::{application, index_store::sync_file};
use harness_domain::{contracts::Request, execution::OperationContext};
use rusqlite::Connection;
use serde_json::json;

#[test]
fn literal_sql_command_explore_reaches_local_procedure_and_table_evidence() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".codegraph")).unwrap();
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    db.execute(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0)",
        [],
    )
    .unwrap();

    std::fs::write(
        root.path().join("schema.sql"),
        concat!(
            "CREATE TABLE dbo.Orders (Id int);\nGO\n",
            "CREATE PROCEDURE dbo.wsp_Find AS SELECT * FROM dbo.Orders;\nGO\n"
        ),
    )
    .unwrap();
    std::fs::write(
        root.path().join("OrdersRepository.cs"),
        concat!(
            "using System.Data.SqlClient;\n",
            "public class OrdersRepository { public void Run() { ",
            "var command = new SqlCommand(\"wsp_Find\", connection); } }\n"
        ),
    )
    .unwrap();
    sync_file(
        root.path(),
        "OrdersRepository.cs",
        &OperationContext::default(),
    )
    .unwrap();

    let call: (String, String) = db
        .query_row(
            "SELECT s.name,t.name FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.name='Run' AND t.kind='stored_procedure'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(call, ("Run".into(), "wsp_Find".into()));

    let table_edge: i64 = db
        .query_row(
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='references' AND s.kind='stored_procedure' AND s.name='wsp_Find' AND t.kind='table' AND t.name='Orders'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(table_edge, 1);

    let target_hashes: (String, String) = db
        .query_row(
            "SELECT json_extract(e.metadata,'$.targetHash'), f.content_hash FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target JOIN files f ON f.path=t.file_path WHERE e.kind='references' AND s.kind='stored_procedure' AND t.kind='table' AND s.name='wsp_Find' AND t.name='Orders'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(target_hashes.0, target_hashes.1, "edge freshness must track the SQL file digest");

    let app = application(root.path()).unwrap();
    let request = serde_json::from_value(json!({
        "query": "Run", "symbol": "Run", "filePath": "OrdersRepository.cs", "view": "full", "maxDepth": 2
    }))
    .unwrap();
    let result = app.execute(Request::Explore(request));
    assert_eq!(result["state"], "ok", "{result}");
    let definitions = result["data"]["definitions"].as_array().unwrap();
    assert!(definitions.iter().any(|definition| {
        definition["symbol"]["kind"] == "stored_procedure"
            && definition["symbol"]["name"] == "wsp_Find"
    }), "{result}");
    assert!(definitions.iter().any(|definition| {
        definition["symbol"]["kind"] == "table" && definition["symbol"]["name"] == "Orders"
    }), "{result}");
}

#[test]
fn unresolved_dynamic_and_ambiguous_sql_command_names_are_not_guessed() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".codegraph")).unwrap();
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    db.execute(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0)",
        [],
    )
    .unwrap();
    std::fs::write(
        root.path().join("a.sql"),
        "CREATE PROCEDURE dbo.Find AS SELECT 1;\nGO\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("b.sql"),
        "CREATE PROCEDURE dbo.Find AS SELECT 2;\nGO\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("Repo.cs"),
        concat!(
            "class Repo { void Run(string name) { ",
            "new SqlCommand(name, connection); ",
            "new SqlCommand(\"Find\", connection); ",
            "new SqlCommand(\"Find\" + suffix, connection); } }"
        ),
    )
    .unwrap();
    sync_file(root.path(), "Repo.cs", &OperationContext::default()).unwrap();
    let edges: i64 = db
        .query_row(
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE s.file_path='Repo.cs' AND t.kind='stored_procedure'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(edges, 0);
}

#[test]
fn sql_file_sync_refreshes_callers_when_procedure_changes_or_is_removed() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".codegraph")).unwrap();
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    db.execute(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0)",
        [],
    )
    .unwrap();

    std::fs::write(
        root.path().join("schema.sql"),
        concat!(
            "CREATE TABLE dbo.Orders (Id int);\nGO\n",
            "CREATE TABLE dbo.Customers (Id int);\nGO\n",
            "CREATE PROCEDURE dbo.wsp_Find AS SELECT * FROM dbo.Orders;\nGO\n"
        ),
    )
    .unwrap();
    std::fs::write(
        root.path().join("OrdersRepository.cs"),
        concat!(
            "using System.Data.SqlClient;\n",
            "public class OrdersRepository { public void Run() { ",
            "var command = new SqlCommand(\"wsp_Find\", connection); } }\n"
        ),
    )
    .unwrap();
    sync_file(
        root.path(),
        "OrdersRepository.cs",
        &OperationContext::default(),
    )
    .unwrap();

    // Exercise the public SQL sync entry point: it must re-resolve the unchanged C# caller.
    std::fs::write(
        root.path().join("schema.sql"),
        concat!(
            "CREATE TABLE dbo.Orders (Id int);\nGO\n",
            "CREATE TABLE dbo.Customers (Id int);\nGO\n",
            "CREATE PROCEDURE dbo.wsp_Find AS SELECT * FROM dbo.Customers;\nGO\n"
        ),
    )
    .unwrap();
    sync_file(root.path(), "schema.sql", &OperationContext::default()).unwrap();

    let changed_dependency: i64 = db
        .query_row(
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='references' AND s.kind='stored_procedure' AND s.name='wsp_Find' AND t.kind='table' AND t.name='Customers'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(changed_dependency, 1, "SQL edits should refresh procedure table dependencies");

    // Removing the procedure should remove its graph target and leave the C# call pending.
    std::fs::write(
        root.path().join("schema.sql"),
        concat!(
            "CREATE TABLE dbo.Orders (Id int);\nGO\n",
            "CREATE TABLE dbo.Customers (Id int);\nGO\n"
        ),
    )
    .unwrap();
    sync_file(root.path(), "schema.sql", &OperationContext::default()).unwrap();

    let stale_call: i64 = db
        .query_row(
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='OrdersRepository.cs' AND t.kind='stored_procedure' AND t.name='wsp_Find'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stale_call, 0, "removed procedure must not remain a resolved call target");
    let pending_call: i64 = db
        .query_row(
            "SELECT count(*) FROM unresolved_refs WHERE file_path='OrdersRepository.cs' AND language='csharp' AND reference_kind='stored_procedure_call' AND reference_name='wsp_Find' AND status='pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending_call, 1, "the C# reference should be pending after its SP is removed");

    // A formerly missing procedure must also trigger re-resolution when a SQL file adds it.
    std::fs::write(
        root.path().join("schema.sql"),
        concat!(
            "CREATE TABLE dbo.Orders (Id int);\nGO\n",
            "CREATE TABLE dbo.Customers (Id int);\nGO\n",
            "CREATE PROCEDURE dbo.wsp_Find AS SELECT * FROM dbo.Orders;\nGO\n"
        ),
    )
    .unwrap();
    sync_file(root.path(), "schema.sql", &OperationContext::default()).unwrap();
    let restored_call: i64 = db
        .query_row(
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='OrdersRepository.cs' AND t.kind='stored_procedure' AND t.name='wsp_Find'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(restored_call, 1, "new procedure should resolve the previously missing C# call");
}
