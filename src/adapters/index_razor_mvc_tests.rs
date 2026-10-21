use super::*;
use crate::adapters::parsers::razor::mvc::collect;

fn routes(source: &str, path: &str) -> Vec<Route> {
    collect(source, path, "view", &[], &OperationContext::default())
        .unwrap()
        .iter()
        .map(|r| serde_json::from_str(r["referenceName"].as_str().unwrap()).unwrap())
        .collect()
}

#[test]
fn literal_helpers_and_tag_helpers_preserve_controller_and_area() {
    let result = routes(
        r#"@Html.ActionLink("Orders", "Index", "Orders")
@Url.Action("Show")
<a asp-action="Edit" asp-controller="Orders" asp-area="">go</a>
<form asp-action='Save' asp-area='Sales'></form>
@Url.Action("List", "Orders", new { area = "Sales", id = 1 })"#,
        "App/Areas/Admin/Views/Home/Index.cshtml",
    );
    assert_eq!(result.len(), 5);
    assert_eq!(result[0].controller, "Orders");
    assert_eq!(result[0].area, "Admin");
    assert_eq!(result[1].controller, "Home");
    assert_eq!(result[2].area, "");
    assert_eq!(result[3].area, "Sales");
    assert_eq!(result[4].area, "Sales");
}

#[test]
fn dynamic_comments_shared_views_shadowed_helpers_and_route_overrides_defer() {
    for source in [
        r#"@Url.Action(action, "Home")"#,
        r#"@Url.Action("Index", controller)"#,
        r#"@Url.Action("Index", "Home", values)"#,
        r#"@* @Url.Action("Index") *@ <!-- <a asp-action="Index"> -->"#,
        r#"<script>const x = '@Url.Action("Index")';</script>"#,
        r#"<a asp-action="@Model.Action" asp-controller="Home">"#,
        r#"<a asp-action="Index" asp-controller=@Model.Controller>"#,
        r#"<a asp-action="Index" asp-route="special">"#,
        r#"<a asp-action="Index" asp-action="Other">"#,
        r#"@{ var Url = custom; } @Url.Action("Index")"#,
        r#"@Url.Action("Index", "Home", new { area = Model.Area })"#,
        r#"@@Url.Action("Index")"#,
        r#"@{ var sample = "<a asp-action='Index'>"; }"#,
    ] {
        assert!(
            routes(source, "Views/Home/Index.cshtml").is_empty(),
            "{source}"
        );
    }
    assert!(routes(r#"@Url.Action("Index")"#, "Views/Shared/_Layout.cshtml").is_empty());
    assert_eq!(
        routes(
            r#"@Url.Action("Index", "Home")"#,
            "Views/Shared/_Layout.cshtml"
        )
        .len(),
        1
    );
}

#[test]
fn mvc_reference_locations_use_original_lines_and_utf16_columns() {
    let source = "@* discarded 😀 *@\n😀 @Url.Action(\"Index\")\n<a asp-action='Edit'>";
    let result = collect(
        source,
        "Views/Home/Index.cshtml",
        "view",
        &[],
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0]["line"], 2);
    assert_eq!(result[0]["column"], 3);
    assert_eq!(result[1]["line"], 3);
    assert_eq!(result[1]["column"], 0);
}

fn database() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE nodes(id TEXT,name TEXT,kind TEXT,qualified_name TEXT,file_path TEXT,language TEXT,visibility TEXT,is_static INTEGER,is_abstract INTEGER,decorators TEXT,signature TEXT,type_parameters TEXT); CREATE INDEX by_name ON nodes(name); CREATE INDEX idx_nodes_lower_name ON nodes(lower(name)); CREATE INDEX by_file ON nodes(file_path);").unwrap();
    db.execute("INSERT INTO nodes VALUES('c','HomeController','class','App::HomeController','Controllers/HomeController.cs','csharp','public',0,0,NULL,NULL,NULL)", []).unwrap();
    db.execute("INSERT INTO nodes VALUES('index','Index','method','App::HomeController::Index','Controllers/HomeController.cs','csharp','public',0,0,NULL,NULL,NULL)", []).unwrap();
    db
}

fn resolve(cache: &mut Snapshot, db: &Connection, action: &str) -> Option<String> {
    let name = serde_json::to_string(&Route {
        root: String::new(),
        area: String::new(),
        controller: "Home".into(),
        action: action.into(),
    })
    .unwrap();
    cache
        .resolve(
            db,
            &name,
            "Views/Home/Index.cshtml",
            &OperationContext::default(),
        )
        .unwrap()
}

#[test]
fn unique_controller_actions_cache_both_hits_and_misses_per_publication() {
    let db = database();
    let mut cache = Snapshot::default();
    for _ in 0..1000 {
        assert_eq!(resolve(&mut cache, &db, "Index").as_deref(), Some("index"));
        assert_eq!(resolve(&mut cache, &db, "Missing"), None);
    }
    assert_eq!(
        cache.queries, 2,
        "one indexed class and file lookup, no per-link SQL"
    );
    assert_eq!(
        cache.parses, 2,
        "one JSON decode per distinct literal route, including misses"
    );
    db.execute("INSERT INTO nodes VALUES('overload','Index','method','App::HomeController::Index','Controllers/HomeController.cs','csharp','public',0,0,NULL,NULL,NULL)", []).unwrap();
    assert_eq!(resolve(&mut Snapshot::default(), &db, "Index"), None);
}

#[test]
fn unrelated_homonyms_non_actions_and_aliases_never_bind() {
    for attribute in ["NonAction", "ActionName(\"Other\")"] {
        let db = database();
        db.execute(
            "UPDATE nodes SET decorators=?1 WHERE id='index'",
            [attribute],
        )
        .unwrap();
        assert_eq!(resolve(&mut Snapshot::default(), &db, "Index"), None);
        if attribute.starts_with("ActionName") {
            assert_eq!(
                resolve(&mut Snapshot::default(), &db, "Other").as_deref(),
                Some("index")
            );
        }
    }
    let db = database();
    db.execute(
        "UPDATE nodes SET qualified_name='App::OtherController::Index' WHERE id='index'",
        [],
    )
    .unwrap();
    assert_eq!(resolve(&mut Snapshot::default(), &db, "Index"), None);
    let context = OperationContext::default();
    context.cancel();
    assert!(
        Snapshot::default()
            .resolve(&db, "{}", "Views/Home/Index.cshtml", &context)
            .is_err()
    );
}

#[test]
fn attribute_route_templates_do_not_change_literal_action_selection() {
    let db = database();
    db.execute(
        "UPDATE nodes SET decorators=?1 WHERE id='c'",
        [r#"["Route(\"[controller]/[action]\")"]"#],
    )
    .unwrap();
    db.execute(
        "UPDATE nodes SET decorators=?1 WHERE id='index'",
        [r#"["HttpGet(\"orders/{id}\")", "Route(\"all\")"]"#],
    )
    .unwrap();
    assert_eq!(
        resolve(&mut Snapshot::default(), &db, "Index").as_deref(),
        Some("index")
    );
    db.execute("UPDATE nodes SET is_abstract=1 WHERE id='c'", [])
        .unwrap();
    assert_eq!(resolve(&mut Snapshot::default(), &db, "Index"), None);
}

#[test]
fn mvc_excludes_generic_static_noncontroller_and_namespaced_nonaction() {
    for update in [
        "UPDATE nodes SET type_parameters='[\"T\"]' WHERE id='index'",
        "UPDATE nodes SET type_parameters='[\"T\"]' WHERE id='c'",
        "UPDATE nodes SET is_static=1 WHERE id='index'",
        "UPDATE nodes SET decorators='Microsoft.AspNetCore.Mvc.NonControllerAttribute' WHERE id='c'",
        "UPDATE nodes SET decorators='Microsoft.AspNetCore.Mvc.NonActionAttribute' WHERE id='index'",
    ] {
        let db = database();
        db.execute(update, []).unwrap();
        assert_eq!(
            resolve(&mut Snapshot::default(), &db, "Index"),
            None,
            "{update}"
        );
    }
    let db = database();
    db.execute(
        "UPDATE nodes SET decorators=?1 WHERE id='index'",
        [r#"["Microsoft.AspNetCore.Mvc.ActionNameAttribute(\"Other\")"]"#],
    )
    .unwrap();
    assert_eq!(
        resolve(&mut Snapshot::default(), &db, "Other").as_deref(),
        Some("index")
    );
    let attrs = r#"["Microsoft.AspNetCore.Mvc.AreaAttribute(\"Admin\")"]"#;
    assert!(controller_attributes(Some(attrs), "Admin"));
    assert!(!controller_attributes(Some(attrs), "Sales"));
    assert!(!controller_attributes(Some("Area(Model.Area)"), "Admin"));
}

#[test]
fn mvc_controller_and_action_case_variants_share_indexed_snapshot() {
    let db = database();
    db.execute("UPDATE nodes SET name='OrdersController',qualified_name='App::OrdersController' WHERE id='c'", []).unwrap();
    db.execute(
        "UPDATE nodes SET qualified_name='App::OrdersController::Index' WHERE id='index'",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE nodes SET file_path='Controllers/OrdersController.cs'",
        [],
    )
    .unwrap();
    let refs = collect(
        r#"@Url.Action("index", "orders") @Url.Action("INDEX", "ORDERS")"#,
        "Views/Orders/Index.cshtml",
        "view",
        &[],
        &OperationContext::default(),
    )
    .unwrap();
    let mut snapshot = Snapshot::default();
    for reference in refs {
        assert_eq!(
            snapshot
                .resolve(
                    &db,
                    reference["referenceName"].as_str().unwrap(),
                    "Views/Orders/Index.cshtml",
                    &OperationContext::default()
                )
                .unwrap()
                .as_deref(),
            Some("index")
        );
    }
    assert_eq!(snapshot.queries, 2);
    let plan: String = db
        .query_row(
            "EXPLAIN QUERY PLAN SELECT id FROM nodes WHERE lower(name)=lower(?1)",
            ["ordersController"],
            |row| row.get(3),
        )
        .unwrap();
    assert!(plan.contains("idx_nodes_lower_name"), "{plan}");
    // A case-folded homonym is still ambiguous, never first-row wins.
    db.execute("INSERT INTO nodes VALUES('overload','index','method','App::OrdersController::index','Controllers/OrdersController.cs','csharp','public',0,0,NULL,NULL,NULL)", []).unwrap();
    let route = Route {
        root: String::new(),
        area: String::new(),
        controller: "orders".into(),
        action: "index".into(),
    };
    assert_eq!(
        Snapshot::default()
            .resolve(
                &db,
                &serde_json::to_string(&route).unwrap(),
                "Views/Orders/Index.cshtml",
                &OperationContext::default()
            )
            .unwrap(),
        None
    );
}
