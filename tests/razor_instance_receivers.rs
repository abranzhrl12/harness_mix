#![cfg(windows)]
use agent_harness::adapters::{application, index_store::sync_file};
use harness_domain::{contracts::Request, execution::OperationContext};
use rusqlite::Connection;
use serde_json::json;

#[test]
fn typed_razor_receivers_respect_lexical_scope_and_deliver_controller_bodies() {
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
        "namespace Demo; public class OrdersController { public void Execute() {} }",
    )
    .unwrap();
    sync_file(
        root.path(),
        "OrdersController.cs",
        &OperationContext::default(),
    )
    .unwrap();
    let source = concat!(
        "@using Demo\n@model Demo.OrdersController\n@inject Demo.OrdersController Injected\n",
        "@functions {\n private OrdersController field;\n",
        " public void Run(OrdersController argument) {\n",
        " argument.Execute();\n field.Execute();\n this.field.Execute();\n",
        " OrdersController local = argument;\n local.Execute();\n",
        " { object local = null; local.Execute(); }\n local.Execute();\n",
        " var inferred = new OrdersController();\n inferred.Execute();\n",
        " dynamic unknown = null; unknown.Execute();\n",
        " System.Action<dynamic> callback = field => field.Execute();\n",
        " }\n}\n@code { public void Inject() { Injected.Execute(); } }\n",
        "<p>😀</p>@Injected.Execute() @(Model.Execute())\n",
        "@{ var rendered = new Demo.OrdersController(); rendered.Execute(); }\n",
        "@rendered.Execute()\n",
        "<script>const fake = '@Injected.Execute()';</script>\n",
        "@* @Injected.Execute() *@ @@Injected.Execute()\n",
    );
    std::fs::write(root.path().join("Index.cshtml"), source).unwrap();
    sync_file(root.path(), "Index.cshtml", &OperationContext::default()).unwrap();
    let count: i64 = db.query_row(
        "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='OrdersController.cs' AND json_extract(e.metadata,'$.resolvedBy')='razor-typed-instance-method-subset-v1'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 11);
    let request = serde_json::from_value(json!({"query":"Run", "symbol":"Run",
        "container":"@functions", "filePath":"Index.cshtml", "view":"full", "direction":"outbound"})).unwrap();
    let result = application(root.path())
        .unwrap()
        .execute(Request::Explore(request));
    assert_eq!(result["state"], "ok", "{result}");
    assert!(
        result["data"]["definitions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["path"] == "OrdersController.cs" && d["symbol"]["name"] == "Execute"),
        "{result}"
    );
}
