#![cfg(windows)]

use agent_harness::adapters::{application, index_store::sync_file};
use harness_domain::{contracts::Request, execution::OperationContext};
use rusqlite::Connection;

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

fn routes(db: &Connection, view: &str) -> Vec<String> {
    db.prepare("SELECT DISTINCT t.file_path FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE s.file_path=?1 AND e.kind='calls' AND t.kind='method' AND t.language='csharp' ORDER BY t.file_path")
        .unwrap().query_map([view], |row| row.get(0)).unwrap()
        .collect::<Result<_, _>>().unwrap()
}

#[test]
fn mvc_routes_do_not_cross_areas_with_identically_named_controllers() {
    let (root, db) = project();
    for path in [
        "Controllers/OrdersController.cs",
        "Areas/Admin/Controllers/OrdersController.cs",
    ] {
        publish(
            root.path(),
            path,
            "public class OrdersController { public void Index() {} }",
        );
    }
    for (view, controller) in [
        (
            "Views/Orders/Index.cshtml",
            "Controllers/OrdersController.cs",
        ),
        (
            "Areas/Admin/Views/Orders/Index.cshtml",
            "Areas/Admin/Controllers/OrdersController.cs",
        ),
    ] {
        publish(
            root.path(),
            view,
            "<a href=\"@Url.Action(\"Index\", \"Orders\")\">Orders</a>",
        );
        assert_eq!(routes(&db, view), vec![controller.to_owned()], "{view}");
    }
}

#[test]
fn mvc_route_candidates_are_recomputed_after_controller_source_changes() {
    let (root, db) = project();
    let controller = "Controllers/OrdersController.cs";
    let view = "Views/Orders/Index.cshtml";
    let source = "<a href=\"@Url.Action(\"Index\", \"Orders\")\">Orders</a>";
    publish(
        root.path(),
        controller,
        "public class OrdersController { public void Index() {} }",
    );
    publish(root.path(), view, source);
    assert_eq!(routes(&db, view), vec![controller.to_owned()]);
    publish(
        root.path(),
        controller,
        "public class OrdersController { public void Other() {} }",
    );
    assert!(
        routes(&db, view).is_empty(),
        "Removed actions must not retain incoming links"
    );
    publish(root.path(), view, &format!("{source}\n"));
    assert!(
        routes(&db, view).is_empty(),
        "A prior candidate cache must not resurrect removed actions"
    );
    publish(
        root.path(),
        controller,
        "public class OrdersController { public void Index() {} public void Index(int id) {} }",
    );
    assert!(
        routes(&db, view).is_empty(),
        "Controller edits must invalidate incoming routes before the view is republished"
    );
    publish(root.path(), view, &format!("{source}\n\n"));
    assert!(
        routes(&db, view).is_empty(),
        "Overloaded actions require conservative ambiguity handling"
    );
}

#[test]
fn mvc_pending_routes_follow_controller_addition_ambiguity_and_deletion() {
    let (root, db) = project();
    let view = "Views/Orders/Index.cshtml";
    let controller = "Controllers/OrdersController.cs";
    let duplicate = "Controllers/OtherOrdersController.cs";
    let controller_source = "public class OrdersController { public void Index() {} }";
    publish(
        root.path(),
        view,
        "<a href=\"@Url.Action(\"Index\", \"Orders\")\">Orders</a>",
    );
    assert!(routes(&db, view).is_empty());
    let pending: i64 = db.query_row(
        "SELECT count(*) FROM unresolved_refs WHERE file_path=?1 AND reference_kind='mvc_route' AND status IN ('pending','failed')",
        [view], |row| row.get(0),
    ).unwrap();
    assert_eq!(pending, 1);
    publish(root.path(), controller, controller_source);
    assert_eq!(
        routes(&db, view),
        vec![controller.to_owned()],
        "Adding a controller must revisit pending routes without republishing the view"
    );
    publish(root.path(), duplicate, controller_source);
    assert!(
        routes(&db, view).is_empty(),
        "A new homonymous controller must invalidate a previously unique route"
    );
    std::fs::remove_file(root.path().join(duplicate)).unwrap();
    sync_file(root.path(), duplicate, &OperationContext::default()).unwrap();
    assert_eq!(
        routes(&db, view),
        vec![controller.to_owned()],
        "Deleting the ambiguous candidate must restore the unique route"
    );
}

#[test]
fn typed_receivers_follow_late_types_and_reject_new_homonyms() {
    let (root, db) = project();
    let view = "Index.cshtml";
    let controller = "OrdersController.cs";
    let duplicate = "OtherOrdersController.cs";
    let source = "namespace Demo; public class OrdersController { public void Execute() {} }";
    publish(
        root.path(),
        view,
        "@using Demo\n@functions { private OrdersController field; public void Run() { field.Execute(); } }",
    );
    assert!(routes(&db, view).is_empty());
    publish(root.path(), controller, source);
    assert_eq!(
        routes(&db, view),
        vec![controller.to_owned()],
        "A late typed target must resolve without republishing the view"
    );
    publish(root.path(), duplicate, source);
    assert!(
        routes(&db, view).is_empty(),
        "A newly ambiguous type must invalidate the prior receiver target"
    );
    std::fs::remove_file(root.path().join(duplicate)).unwrap();
    sync_file(root.path(), duplicate, &OperationContext::default()).unwrap();
    assert_eq!(routes(&db, view), vec![controller.to_owned()]);
}

#[test]
fn razor_file_selector_delivers_inline_mvc_controller_in_one_exploration() {
    let (root, _db) = project();
    publish(
        root.path(),
        "Controllers/OrdersController.cs",
        "public class OrdersController { public void Index() {} }",
    );
    publish(
        root.path(),
        "Views/Orders/Index.cshtml",
        "<a href=\"@Url.Action(\"Index\", \"Orders\")\">Orders</a>",
    );
    let app = application(root.path()).unwrap();
    let args = serde_json::from_value(serde_json::json!({
        "filePath":"Views/Orders/Index.cshtml", "view":"full", "direction":"outbound"
    }))
    .unwrap();
    let result = app.execute(Request::Explore(args));
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["focus"]["status"], "resolved", "{result}");
    assert_eq!(
        result["data"]["unresolved"],
        serde_json::json!([]),
        "{result}"
    );
    assert!(
        result["data"]["definitions"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |definition| definition["path"] == "Controllers/OrdersController.cs"
                    && definition["symbol"]["name"] == "Index"
            ),
        "{result}"
    );
    for input in [
        serde_json::json!({"filePath":"Controllers/OrdersController.cs"}),
        serde_json::json!({"filePath":"Views/Orders/Index.cshtml", "view":"impact"}),
        serde_json::json!({"filePath":"Views/Orders/Index.cshtml", "container":"@code"}),
    ] {
        let result = app.execute(Request::Explore(serde_json::from_value(input).unwrap()));
        assert_eq!(result["state"], "error", "{result}");
        assert_eq!(result["diagnostics"][0]["code"], "ARGUMENT", "{result}");
    }
}
