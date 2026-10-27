#![cfg(windows)]

use agent_harness::adapters::{application, index_store::sync_file};
use harness_domain::{contracts::Request, execution::OperationContext};
use rusqlite::Connection;
use serde_json::{Value, json};

fn project() -> (tempfile::TempDir, Connection) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".codegraph")).unwrap();
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    db.pragma_update(None, "journal_mode", "WAL").unwrap();
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0)",
        [],
    )
    .unwrap();
    (root, db)
}

fn publish(root: &std::path::Path, path: &str, source: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, source).unwrap();
    sync_file(root, path, &OperationContext::default()).unwrap();
}

fn http_edges(db: &Connection) -> Vec<(String, String, i64, Value)> {
    let mut statement = db
        .prepare(
            "SELECT s.file_path, t.name, t.start_line, e.metadata
             FROM edges e
             JOIN nodes s ON s.id=e.source
             JOIN nodes t ON t.id=e.target
             WHERE e.kind='calls' AND json_extract(e.metadata, '$.transport')='http'
             ORDER BY s.file_path, t.name, t.start_line",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            let metadata: String = row.get(3)?;
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                serde_json::from_str(&metadata).unwrap(),
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn explore_symbol(
    root: &std::path::Path,
    file_path: &str,
    symbol: Option<&str>,
    direction: &str,
) -> Value {
    let app = application(root).unwrap();
    let mut input = json!({
        "filePath": file_path,
        "view": "full",
        "direction": direction
    });
    if let Some(symbol) = symbol {
        input["symbol"] = json!(symbol);
    }
    let args = serde_json::from_value(input).unwrap();
    app.execute(Request::Explore(args))
}

const CONTROLLER: &str = r#"
using Microsoft.AspNetCore.Mvc;

[Route("api/orders")]
public class OrdersController : ControllerBase
{
    [HttpGet("list")]
    public IActionResult List() => Ok();

    [HttpPost("save")]
    public IActionResult Save() => Ok();

    [HttpGet("Recent")]
    public IActionResult Recent() => Ok();

    [HttpDelete("item")]
    public IActionResult Delete() => Ok();
}
"#;

#[test]
fn frontend_requests_link_to_controller_routes_and_explore_both_directions() {
    let (root, db) = project();
    publish(root.path(), "Controllers/OrdersController.cs", CONTROLLER);
    publish(
        root.path(),
        "wwwroot/orders.js",
        r#"
function loadOrders() {
    fetch("/api/orders/list");
    fetch("/api/orders/save", { method: "GET" });
    axios.post("/api/orders/save", {});
    $.get("/api/orders/Recent");
    $.ajax({ url: "/api/orders/item", type: "DELETE" });
}
"#,
    );
    publish(
        root.path(),
        "src/Orders.vue",
        r#"<template><div /></template>
<script>
export default {
  mounted() {
    axios.delete("/api/orders/item");
  }
}
</script>
"#,
    );

    let edges = http_edges(&db);
    let routes = edges
        .iter()
        .map(|(_, action, _, metadata)| {
            let reference: Value =
                serde_json::from_str(metadata["refName"].as_str().unwrap()).unwrap();
            (
                action.clone(),
                reference["url"].as_str().unwrap().to_owned(),
                reference["method"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    for expected in [
        ("List", "/api/orders/list", "GET"),
        ("Save", "/api/orders/save", "POST"),
        ("Recent", "/api/orders/Recent", "GET"),
        ("Delete", "/api/orders/item", "DELETE"),
    ] {
        assert!(
            routes.iter().any(|route| route
                == &(
                    expected.0.to_owned(),
                    expected.1.to_owned(),
                    expected.2.to_owned()
                )),
            "missing route {expected:?}: {routes:?}"
        );
    }
    assert!(
        !edges.iter().any(|(_, action, _, metadata)| {
            if action != "Save" {
                return false;
            }
            let reference: Value =
                serde_json::from_str(metadata["refName"].as_str().unwrap()).unwrap();
            reference["url"] == "/api/orders/save" && reference["method"] == "GET"
        }),
        "a GET request must not link to a POST-only endpoint: {edges:?}"
    );
    assert!(
        edges.iter().any(|(path, action, _, metadata)| {
            path == "src/Orders.vue" && action == "Delete" && metadata["refKind"] == "http_route"
        }),
        "Vue inline-script requests should resolve: {edges:?}"
    );

    let outbound = explore_symbol(
        root.path(),
        "wwwroot/orders.js",
        Some("loadOrders"),
        "outbound",
    );
    assert_eq!(outbound["state"], "ok", "{outbound}");
    let graph = &outbound["data"]["graph"];
    let list_id = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| {
            node["name"] == "List" && node["filePath"] == "Controllers/OrdersController.cs"
        })
        .unwrap()["id"]
        .clone();
    assert!(
        graph["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| { edge["kind"] == "calls" && edge["target"] == list_id }),
        "outbound Explore should expose the HTTP route edge: {outbound}"
    );

    let inbound = explore_symbol(
        root.path(),
        "Controllers/OrdersController.cs",
        Some("List"),
        "inbound",
    );
    assert_eq!(inbound["state"], "ok", "{inbound}");
    let graph = &inbound["data"]["graph"];
    let caller = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["name"] == "loadOrders" && node["filePath"] == "wwwroot/orders.js")
        .unwrap();
    let target = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| {
            node["name"] == "List" && node["filePath"] == "Controllers/OrdersController.cs"
        })
        .unwrap();
    assert_eq!(caller["startLine"], 2, "{inbound}");
    assert_eq!(target["startLine"], 7, "{inbound}");
    assert!(
        graph["edges"].as_array().unwrap().iter().any(|edge| {
            edge["kind"] == "calls"
                && edge["source"] == caller["id"]
                && edge["target"] == target["id"]
                && edge["line"] == 3
                && edge["column"] == 4
                && edge["transport"] == "http"
                && edge["httpMethod"] == "GET"
        }),
        "inbound Explore focused on the action should include the frontend symbol, file, and line: {inbound}"
    );
    let impact = serde_json::from_value(json!({
        "filePath": "Controllers/OrdersController.cs",
        "symbol": "List",
        "view": "impact",
        "direction": "inbound"
    }))
    .unwrap();
    let impact = application(root.path())
        .unwrap()
        .execute(Request::Explore(impact));
    assert_eq!(impact["state"], "ok", "{impact}");
    assert!(
        impact["data"]["callers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|caller| {
                caller["symbol"] == "loadOrders"
                    && caller["file"] == "wwwroot/orders.js"
                    && caller["declarationLine"] == 2
                    && caller["direct"] == true
            }),
        "impact should report the frontend caller, file, and declaration line: {impact}"
    );
    assert_eq!(impact["data"]["counts"]["directCallers"], 1, "{impact}");
}

#[test]
fn route_links_refresh_after_action_edits_ambiguity_and_controller_deletion() {
    let (root, db) = project();
    let controller_path = "Controllers/OrdersController.cs";
    let duplicate_path = "Controllers/ArchiveController.cs";
    let client_path = "wwwroot/orders.js";
    publish(root.path(), controller_path, CONTROLLER);
    publish(root.path(), client_path, "fetch(\"/api/orders/list\");");

    let list_edges = |db: &Connection| {
        http_edges(db)
            .into_iter()
            .filter(|(path, action, _, metadata)| {
                path == client_path
                    && action == "List"
                    && metadata["refName"]
                        .as_str()
                        .is_some_and(|name| name.contains("/api/orders/list"))
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(list_edges(&db).len(), 1);

    // Changing the route declaration removes the old endpoint after republishing.
    let changed = CONTROLLER.replace("[HttpGet(\"list\")]", "[HttpGet(\"browse\")]");
    publish(root.path(), controller_path, &changed);
    assert!(
        list_edges(&db).is_empty(),
        "an edited action must not retain its old route edge: {:?}",
        http_edges(&db)
    );

    publish(root.path(), controller_path, CONTROLLER);
    assert_eq!(list_edges(&db).len(), 1);
    publish(
        root.path(),
        duplicate_path,
        r#"
using Microsoft.AspNetCore.Mvc;
[Route("api/orders")]
public class ArchiveController : ControllerBase
{
    [HttpGet("list")]
    public IActionResult List() => Ok();
}
"#,
    );
    assert!(
        list_edges(&db).is_empty(),
        "duplicate endpoint candidates must remain unresolved: {:?}",
        http_edges(&db)
    );

    std::fs::remove_file(root.path().join(duplicate_path)).unwrap();
    sync_file(root.path(), duplicate_path, &OperationContext::default()).unwrap();
    assert_eq!(list_edges(&db).len(), 1);

    publish(
        root.path(),
        client_path,
        "fetch(\"/api/orders/save\", { method: \"POST\" });",
    );
    assert!(
        list_edges(&db).is_empty(),
        "editing a frontend caller must remove its old route edge"
    );
    assert!(
        http_edges(&db)
            .iter()
            .any(|(path, action, _, _)| path == client_path && action == "Save"),
        "the edited POST request should link to Save: {:?}",
        http_edges(&db)
    );

    std::fs::remove_file(root.path().join(client_path)).unwrap();
    sync_file(root.path(), client_path, &OperationContext::default()).unwrap();
    assert!(
        http_edges(&db)
            .iter()
            .all(|(path, _, _, _)| path != client_path),
        "deleting a frontend caller must remove its outgoing route edge"
    );

    publish(root.path(), client_path, "fetch(\"/api/orders/list\");");
    assert_eq!(list_edges(&db).len(), 1);
    std::fs::remove_file(root.path().join(controller_path)).unwrap();
    sync_file(root.path(), controller_path, &OperationContext::default()).unwrap();
    assert!(
        list_edges(&db).is_empty(),
        "deleting the only route candidate must remove its incoming HTTP edge"
    );
}

#[test]
fn conventional_routes_resolve_late_backends_and_keep_pending_requests_unlinked() {
    let (root, db) = project();
    let client = "src/taller.js";
    publish(
        root.path(),
        client,
        "fetch(\"/Taller/NumeroOrden_Listar\");",
    );
    assert!(http_edges(&db).is_empty());
    let pending: i64 = db
        .query_row(
            "SELECT count(*) FROM unresolved_refs
             WHERE reference_kind='http_route' AND reference_name LIKE '%NumeroOrden_Listar%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        pending, 1,
        "unresolved literal requests should remain pending"
    );

    publish(
        root.path(),
        "Controllers/TallerController.cs",
        r#"
using Microsoft.AspNetCore.Mvc;
public class TallerController : ControllerBase
{
    [HttpGet]
    public IActionResult NumeroOrden_Listar() => Ok();
}
"#,
    );
    let resolved = http_edges(&db);
    assert_eq!(resolved.len(), 1, "{resolved:?}");
    let (path, action, _, metadata) = &resolved[0];
    assert_eq!(path, client);
    assert_eq!(action, "NumeroOrden_Listar");
    let reference: Value = serde_json::from_str(metadata["refName"].as_str().unwrap()).unwrap();
    assert_eq!(reference["url"], "/Taller/NumeroOrden_Listar");
    assert_eq!(reference["method"], "GET");
}
