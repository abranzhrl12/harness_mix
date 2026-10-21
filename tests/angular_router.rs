#![cfg(windows)]

use agent_harness::adapters::index_store::sync_file;
use harness_domain::execution::OperationContext;
use rusqlite::Connection;
use std::{fs, path::Path};

const ENGINE: &str = "angular-route-component-subset-v1";

fn fixture(root: &Path) {
    fs::create_dir_all(root.join(".codegraph")).unwrap();
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0)",
        [],
    )
    .unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL").unwrap();
}

fn sync(root: &Path, path: &str) -> serde_json::Value {
    sync_file(root, path, &OperationContext::default()).unwrap()
}

fn count(root: &Path, query: &str) -> i64 {
    Connection::open(root.join(".codegraph/codegraph.db"))
        .unwrap()
        .query_row(query, [], |row| row.get(0))
        .unwrap()
}

#[test]
fn registered_nested_routes_bind_only_the_unique_imported_component_and_retry_incrementally() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("app.routes.ts"),
        concat!(
            "import type { Routes } from '@angular/router';\n",
            "import { RouterModule } from '@angular/router';\n",
            "import { Home as Landing } from './home';\n",
            "const routes: Routes = [{ path: 'home', component: Landing }];\n",
            "@NgModule({ imports: [RouterModule.forRoot(routes)] }) export class AppModule {}\n"
        ),
    )
    .unwrap();

    let first = sync(root.path(), "app.routes.ts");
    assert_eq!(first["committed"], true, "{first}");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_kind='angular_route_component' AND reference_name='Landing' AND status='pending'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        0
    );

    fs::write(root.path().join("home.ts"), "export class Home {}\n").unwrap();
    let added = sync(root.path(), "home.ts");
    assert_eq!(added["committed"], true, "{added}");
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='references' AND s.file_path='app.routes.ts' AND t.file_path='home.ts' AND t.name='Home' AND json_extract(e.metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        1
    );

    fs::write(root.path().join("next.ts"), "export class Next {}\n").unwrap();
    let next_added = sync(root.path(), "next.ts");
    assert_eq!(next_added["committed"], true, "{next_added}");
    fs::write(
        root.path().join("app.routes.ts"),
        concat!(
            "import type { Routes } from '@angular/router';\n",
            "import { RouterModule } from '@angular/router';\n",
            "import { Next as Landing } from './next';\n",
            "const routes: Routes = [{ path: 'home', component: Landing }];\n",
            "@NgModule({ imports: [RouterModule.forRoot(routes)] }) export class AppModule {}\n"
        ),
    )
    .unwrap();
    let edited_importer = sync(root.path(), "app.routes.ts");
    assert_eq!(edited_importer["committed"], true, "{edited_importer}");
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE t.file_path='home.ts' AND json_extract(e.metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE t.file_path='next.ts' AND json_extract(e.metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        1
    );

    fs::remove_file(root.path().join("next.ts")).unwrap();
    let deleted = sync(root.path(), "next.ts");
    assert_eq!(deleted["committed"], true, "{deleted}");
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_kind='angular_route_component' AND status='pending'"
        ),
        1
    );

    fs::write(root.path().join("next.ts"), "export class Next {}\n").unwrap();
    let readded = sync(root.path(), "next.ts");
    assert_eq!(readded["committed"], true, "{readded}");
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        1
    );
}

#[test]
fn angular_route_resolver_does_not_fall_back_to_a_global_same_name_class() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    for (path, source) in [
        (
            "app.routes.ts",
            "import type { Routes } from '@angular/router';\nimport { RouterModule } from '@angular/router';\nimport { Home } from './missing';\nconst routes: Routes = [{path:'home', component:Home}];\nRouterModule.forRoot(routes);\n",
        ),
        ("decoy.ts", "export class Home {}\n"),
    ] {
        fs::write(root.path().join(path), source).unwrap();
        let result = sync(root.path(), path);
        assert_eq!(result["committed"], true, "{path}: {result}");
    }
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_kind='angular_route_component' AND status='pending'"
        ),
        1
    );
}

#[test]
fn type_only_component_bindings_and_forged_route_sites_stay_pending() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("app.routes.ts"),
        concat!(
            "import type { Routes } from '@angular/router';\n",
            "import { RouterModule } from '@angular/router';\n",
            "import type { Home } from './home';\n",
            "const routes: Routes = [{ path: 'home', component: Home }];\n",
            "RouterModule.forRoot(routes);\n"
        ),
    )
    .unwrap();
    fs::write(root.path().join("home.ts"), "export class Home {}\n").unwrap();
    for path in ["app.routes.ts", "home.ts"] {
        let result = sync(root.path(), path);
        assert_eq!(result["committed"], true, "{path}: {result}");
    }
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_kind='angular_route_component' AND status='pending'"
        ),
        1
    );

    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    db.execute(
        "UPDATE unresolved_refs SET from_node_id=(SELECT id FROM nodes WHERE file_path='app.routes.ts' AND kind='file' LIMIT 1) WHERE reference_kind='angular_route_component'",
        [],
    )
    .unwrap();
    fs::write(
        root.path().join("home.ts"),
        "export class Home { value = 1; }\n",
    )
    .unwrap();
    let changed_target = sync(root.path(), "home.ts");
    assert_eq!(changed_target["committed"], true, "{changed_target}");
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_kind='angular_route_component' AND status='pending'"
        ),
        1
    );
}
