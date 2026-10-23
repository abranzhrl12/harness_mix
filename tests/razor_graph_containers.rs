#![cfg(windows)]

use agent_harness::adapters::{application, index_store::sync_file};
use harness_domain::{contracts::Request, execution::OperationContext};
use rusqlite::Connection;
use serde_json::json;

#[test]
fn razor_graph_keeps_code_and_functions_selectors_from_the_live_outline() {
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
    // Same-line blocks and astral markup exercise the native UTF-16 columns.
    let source = "<p>😀</p>@functions { public void Run() { Helper(); } } @code { public void Helper() {} }\r\n";
    std::fs::write(root.path().join("Index.cshtml"), source).unwrap();
    sync_file(root.path(), "Index.cshtml", &OperationContext::default()).unwrap();
    let app = application(root.path()).unwrap();
    for (symbol, container) in [("Run", "@functions"), ("Helper", "@code")] {
        let qualified: String = db
            .query_row(
                "SELECT qualified_name FROM nodes WHERE name=?1",
                [symbol],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(qualified, format!("{container}.{symbol}"));
        let request = serde_json::from_value(json!({
            "query": symbol, "symbol": symbol, "filePath": "Index.cshtml",
            "container": container, "view": "full"
        }))
        .unwrap();
        let result = app.execute(Request::Explore(request));
        assert_eq!(result["state"], "ok", "{result}");
        assert_eq!(result["data"]["unresolved"], json!([]), "{result}");
        assert!(
            result["data"]["definitions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|definition| definition["role"] == "target"
                    && definition["symbol"]["name"] == symbol),
            "{result}"
        );
    }
    let calls: i64 = db.query_row(
        "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.name='Run' AND t.name='Helper'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(calls, 1);
}

#[test]
fn razor_explore_reaches_an_absolute_public_static_controller_method() {
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
        root.path().join("OrdersController.cs"),
        concat!(
            "namespace Demo; public class OrdersController { ",
            "public static void Render() {} public void Instance() {} ",
            "public static void Ambiguous(int value) {} ",
            "public static void Ambiguous(string value) {} }"
        ),
    )
    .unwrap();
    sync_file(
        root.path(),
        "OrdersController.cs",
        &OperationContext::default(),
    )
    .unwrap();
    std::fs::write(
        root.path().join("Index.cshtml"),
        concat!(
            "@code { public void Run() { global::Demo.OrdersController.Render(); ",
            "global::Demo.OrdersController.Instance(); ",
            "global::Demo.OrdersController.Ambiguous(1); } }"
        ),
    )
    .unwrap();
    sync_file(root.path(), "Index.cshtml", &OperationContext::default()).unwrap();
    let calls: Vec<String> = db.prepare(
        "SELECT t.name FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='Index.cshtml' AND t.file_path='OrdersController.cs' ORDER BY t.name"
    ).unwrap().query_map([], |row| row.get(0)).unwrap()
        .collect::<Result<_, _>>().unwrap();
    assert_eq!(calls, vec!["Render"]);
    let app = application(root.path()).unwrap();
    let request = serde_json::from_value(json!({
        "query": "Run", "symbol": "Run", "container": "@code",
        "filePath": "Index.cshtml", "view": "full", "direction": "outbound"
    }))
    .unwrap();
    let result = app.execute(Request::Explore(request));
    assert_eq!(result["state"], "ok", "{result}");
    assert!(
        result["data"]["definitions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|definition| definition["path"] == "OrdersController.cs"
                && definition["symbol"]["name"] == "Render"),
        "{result}"
    );
}

#[test]
fn razor_url_content_resolves_to_an_indexed_script_file() {
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
    std::fs::create_dir_all(root.path().join("Scripts")).unwrap();
    std::fs::write(root.path().join("Scripts/app.js"), "function boot() {}\n").unwrap();
    sync_file(root.path(), "Scripts/app.js", &OperationContext::default()).unwrap();
    std::fs::create_dir_all(root.path().join("Views/Home")).unwrap();
    std::fs::write(
        root.path().join("Views/Home/Index.cshtml"),
        "<script src=\"@Url.Content(\"~/Scripts/app.js\")\"></script>",
    )
    .unwrap();
    sync_file(
        root.path(),
        "Views/Home/Index.cshtml",
        &OperationContext::default(),
    )
    .unwrap();
    let links: i64 = db
        .query_row(
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='references' AND s.file_path='Views/Home/Index.cshtml' AND t.file_path='Scripts/app.js'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(links, 1);
}
