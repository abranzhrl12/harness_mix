#![cfg(windows)]

use agent_harness::adapters::index_store::sync_file;
use harness_domain::execution::OperationContext;
use rusqlite::Connection;
use std::fs;

const LAYOUT: &str = "<h1>@ViewBag.Title</h1>\n@RenderSection(\"Scripts\", required: false)\n";
const PARTIAL: &str = "@model Legacy.Models.Order\n<table></table>\n";
const ORDERS: &str = r#"@model Legacy.Models.Order
@{
    ViewBag.Title = "Index";
    Layout = "~/Views/Shared/_Layout.cshtml";
}
@helper Row(Order o) {
    <tr><td>@o.Id</td></tr>
}
@section Scripts {
    <script>if (1 < 2) { alert("x"); }</script>
}
<table>
    <tr><td><input type="text" value='@Url.Content("~/Content/logo.png")' /></td></tr>
</table>
<a href="@Url.Action("Edit", "Orders")">Edit</a>
@Html.ActionLink("Home", "Index", "Home")
@Html.Partial("_Orders")
@{ Html.RenderPartial("_Orders"); }
@{ Html.RenderAction("Details", "Orders"); }
"#;
const CONTROLLER: &str = r#"namespace Legacy.Controllers
{
    public class OrdersController : Controller
    {
        public ActionResult Index() => View();
        public ActionResult Edit(int id) => View();
        public ActionResult Details(int id) => View();
    }
    public class HomeController : Controller
    {
        public ActionResult Index() => View();
    }
}
"#;

fn edges(db: &Connection) -> Vec<(String, String, String)> {
    let mut statement = db
        .prepare(
            "SELECT s.file_path, t.file_path, e.kind FROM edges e \
             JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target \
             WHERE s.language='razor' AND t.language='razor' ORDER BY s.file_path, t.file_path",
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap();
    rows.map(|row| row.unwrap()).collect()
}

fn pending(db: &Connection) -> Vec<(String, String)> {
    let mut statement = db
        .prepare(
            "SELECT reference_kind, reference_name FROM unresolved_refs \
             WHERE language='razor' ORDER BY reference_kind, reference_name",
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap();
    rows.map(|row| row.unwrap()).collect()
}

#[test]
fn legacy_mvc5_view_extracts_partial_layout_and_action_references() {
    use agent_harness::adapters::index_extract::extract;
    let result = extract(
        "Views/Orders/Index.cshtml",
        ORDERS,
        &OperationContext::default(),
    )
    .unwrap();
    let references = result["unresolvedReferences"].as_array().unwrap();
    let kinds = references
        .iter()
        .map(|reference| {
            (
                reference["referenceKind"].as_str().unwrap().to_owned(),
                reference["referenceName"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    println!("{kinds:?}");
    assert!(
        kinds
            .iter()
            .any(|(kind, name)| kind == "razor_partial" && name == "_Orders"),
        "{kinds:?}"
    );
    assert!(
        kinds
            .iter()
            .any(|(kind, name)| kind == "razor_layout" && name == "~/Views/Shared/_Layout.cshtml"),
        "{kinds:?}"
    );
    // Two action routes: `Url.Action` and `Html.ActionLink` in markup, plus
    // `Html.RenderAction` inside an inline C# region.
    let routes = kinds
        .iter()
        .filter(|(kind, _)| kind == "mvc_route")
        .map(|(_, name)| name.clone())
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 3, "{routes:?}");
    assert!(
        routes
            .iter()
            .any(|name| name.contains("\"action\":\"Edit\""))
    );
    assert!(
        routes
            .iter()
            .any(|name| name.contains("\"action\":\"Details\""))
    );
    assert!(
        routes
            .iter()
            .any(|name| name.contains("\"action\":\"Index\""))
    );
    // The C# bridge no longer duplicates the MVC helpers.
    assert!(
        !kinds
            .iter()
            .any(|(kind, name)| kind == "calls" && name.starts_with("Html.")),
        "{kinds:?}"
    );
}

#[test]
fn legacy_mvc5_view_links_partial_layout_and_actions() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".codegraph")).unwrap();
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    db.execute(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0)",
        [],
    )
    .unwrap();
    fs::create_dir_all(root.path().join("Views/Shared")).unwrap();
    fs::create_dir_all(root.path().join("Views/Orders")).unwrap();
    fs::create_dir_all(root.path().join("Controllers")).unwrap();
    fs::write(root.path().join("Views/Shared/_Layout.cshtml"), LAYOUT).unwrap();
    fs::write(root.path().join("Views/Shared/_Orders.cshtml"), PARTIAL).unwrap();
    fs::write(root.path().join("Views/Orders/Index.cshtml"), ORDERS).unwrap();
    fs::write(
        root.path().join("Controllers/OrdersController.cs"),
        CONTROLLER,
    )
    .unwrap();
    let context = OperationContext::default();
    // Targets are published first: a view reference resolves against the
    // indexed view files of the same publication.
    for path in [
        "Views/Shared/_Layout.cshtml",
        "Views/Shared/_Orders.cshtml",
        "Controllers/OrdersController.cs",
    ] {
        sync_file(root.path(), path, &context).unwrap();
    }
    sync_file(root.path(), "Views/Orders/Index.cshtml", &context).unwrap();
    let found = edges(&db);
    for (from, to, kind) in &found {
        println!("{from} -> {to} ({kind})");
    }
    // The view reaches its layout and the shared partial.
    assert!(
        found
            .iter()
            .any(|(from, to, kind)| from == "Views/Orders/Index.cshtml"
                && to == "Views/Shared/_Layout.cshtml"
                && kind == "references"),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|(from, to, kind)| from == "Views/Orders/Index.cshtml"
                && to == "Views/Shared/_Orders.cshtml"
                && kind == "references"),
        "{found:?}"
    );
    // Controller actions reached from the view are resolved as calls.
    let mut statement = db
        .prepare(
            "SELECT t.file_path, t.name FROM edges e JOIN nodes s ON s.id=e.source \
             JOIN nodes t ON t.id=e.target WHERE s.file_path='Views/Orders/Index.cshtml' \
             AND e.kind='calls' ORDER BY t.name",
        )
        .unwrap();
    let actions = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .map(|row| row.unwrap())
        .collect::<Vec<_>>();
    println!("actions: {actions:?}");
    for expected in ["Details", "Edit", "Index"] {
        assert!(
            actions
                .iter()
                .any(|(file, name)| file == "Controllers/OrdersController.cs" && name == expected),
            "{actions:?}"
        );
    }
    println!("pending: {:?}", pending(&db));
}
