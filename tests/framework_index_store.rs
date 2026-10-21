#![cfg(windows)]

use agent_harness::adapters::index_store::{sync_file, sync_file_forced};
use harness_domain::execution::OperationContext;
use rusqlite::Connection;
use std::{fs, path::Path};

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

fn db(root: &Path) -> Connection {
    Connection::open(root.join(".codegraph/codegraph.db")).unwrap()
}

fn sync(root: &Path, path: &str) -> serde_json::Value {
    sync_file(root, path, &OperationContext::default()).unwrap()
}

fn template_edge_count(root: &Path) -> i64 {
    db(root)
        .query_row(
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='references' AND s.kind='component' AND t.kind='file' AND json_extract(e.metadata,'$.refKind')='framework_template'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn angular_template_reference_resolves_rechecks_hashes_and_remains_pending_on_delete() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("app.html"), "<main>first</main>\n").unwrap();
    fs::write(
        root.path().join("app.component.ts"),
        "import { Component } from '@angular/core';\n@Component({ templateUrl: './app.html' })\nexport class AppComponent {}\n",
    )
    .unwrap();

    let template = sync(root.path(), "app.html");
    assert_eq!(template["committed"], true, "{template}");
    assert_eq!(
        template["extractionCoverage"]["parser"],
        "html-structural-subset-v1"
    );
    assert!(
        template["extractionCoverage"]["limitations"][0]
            .as_str()
            .unwrap()
            .contains("no Angular context")
    );
    let published = sync(root.path(), "app.component.ts");
    assert_eq!(published["committed"], true, "{published}");
    assert_eq!(template_edge_count(root.path()), 1);
    let resolved: (String, String, String) = db(root.path())
        .query_row(
            "SELECT e.metadata,s.id,t.id FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='references' AND json_extract(e.metadata,'$.refKind')='framework_template'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(resolved.1.starts_with("native:framework:"));
    assert_eq!(resolved.2, "file:app.html");
    let metadata: serde_json::Value = serde_json::from_str(&resolved.0).unwrap();
    assert_eq!(metadata["refName"], "app.html");
    assert!(metadata["sourceHash"].as_str().is_some());
    assert!(metadata["targetHash"].as_str().is_some());
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM unresolved_refs WHERE reference_kind='framework_template'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );

    fs::write(root.path().join("app.html"), "<main>updated</main>\n").unwrap();
    let refreshed = sync(root.path(), "app.html");
    assert_eq!(refreshed["committed"], true, "{refreshed}");
    assert_eq!(template_edge_count(root.path()), 1);
    let current_hash: String = db(root.path())
        .query_row(
            "SELECT json_extract(e.metadata,'$.targetHash') FROM edges e WHERE json_extract(e.metadata,'$.refKind')='framework_template'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        current_hash,
        harness_domain::hash(b"<main>updated</main>\n")
    );

    // A currently edited component cannot reuse the old indexed template
    // association when only the target file is republished.
    fs::write(
        root.path().join("app.component.ts"),
        "import { Component } from '@angular/core';\n@Component({ templateUrl: './other.html' })\nexport class AppComponent {}\n",
    )
    .unwrap();
    fs::write(root.path().join("app.html"), "<main>third</main>\n").unwrap();
    let stale_source = sync(root.path(), "app.html");
    assert_eq!(stale_source["committed"], true, "{stale_source}");
    assert_eq!(template_edge_count(root.path()), 0);
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM unresolved_refs WHERE reference_kind='framework_template' AND status='pending' AND reference_name='app.html'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );

    fs::remove_file(root.path().join("app.html")).unwrap();
    let deleted = sync(root.path(), "app.html");
    assert_eq!(deleted["deleted"], true, "{deleted}");
    assert_eq!(template_edge_count(root.path()), 0);
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM unresolved_refs WHERE reference_kind='framework_template' AND status='pending' AND reference_name='app.html'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn component_published_before_template_is_retried_when_html_is_indexed() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("app.component.ts"),
        "import { Component } from '@angular/core';\n@Component({ templateUrl: './app.htm' })\nexport class AppComponent {}\n",
    )
    .unwrap();
    let component = sync(root.path(), "app.component.ts");
    assert_eq!(component["committed"], true, "{component}");
    assert_eq!(template_edge_count(root.path()), 0);
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM unresolved_refs WHERE reference_kind='framework_template' AND status='pending' AND reference_name='app.htm'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );

    fs::write(root.path().join("app.htm"), "<main>ready</main>\n").unwrap();
    let template = sync(root.path(), "app.htm");
    assert_eq!(template["committed"], true, "{template}");
    assert_eq!(template_edge_count(root.path()), 1);
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM unresolved_refs WHERE reference_kind='framework_template' AND status='pending'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn unsafe_angular_template_url_is_not_published_as_a_graph_relation() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir_all(root.path().join("src")).unwrap();
    fs::write(
        root.path().join("src/app.component.ts"),
        "import { Component } from '@angular/core';\n@Component({ templateUrl: '../../outside.html' })\nexport class AppComponent {}\n",
    )
    .unwrap();
    let component = sync(root.path(), "src/app.component.ts");
    assert_eq!(component["committed"], true, "{component}");
    assert_eq!(template_edge_count(root.path()), 0);
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM unresolved_refs WHERE reference_kind='framework_template'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn native_framework_component_ids_are_replaced_but_provider_components_survive() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("app.html"), "<main>template</main>\n").unwrap();
    fs::write(
        root.path().join("app.component.ts"),
        "import { Component } from '@angular/core';\n@Component({ templateUrl: './app.html' })\nexport class AppComponent {}\n",
    )
    .unwrap();
    sync(root.path(), "app.html");

    let connection = db(root.path());
    connection
        .execute(
            "INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES('provider:app-component','component','AppComponent','AppComponent','app.component.ts','typescript',3,3,0,0,0)",
            [],
        )
        .unwrap();
    drop(connection);

    let first = sync(root.path(), "app.component.ts");
    assert_eq!(first["committed"], true, "{first}");
    assert_eq!(first["providerNodesPreserved"], 1);
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM nodes WHERE id='provider:app-component' AND kind='component'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM nodes WHERE file_path='app.component.ts' AND kind='component' AND id LIKE 'native:framework:%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );

    fs::write(
        root.path().join("app.component.ts"),
        "import { Component } from '@angular/core';\n@Component({ templateUrl: './missing.html' })\nexport class AppComponent {}\n",
    )
    .unwrap();
    let changed = sync_file_forced(
        root.path(),
        "app.component.ts",
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(changed["committed"], true, "{changed}");
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM nodes WHERE file_path='app.component.ts' AND kind='component' AND id LIKE 'native:framework:%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM unresolved_refs WHERE reference_kind='framework_template' AND reference_name='missing.html' AND status='pending'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM nodes WHERE id='provider:app-component'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn vue_framework_nodes_are_replaced_on_resync_and_removed_on_delete() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("helper.ts"),
        "export function helper() {}\n",
    )
    .unwrap();
    sync(root.path(), "helper.ts");
    fs::write(
        root.path().join("Card.vue"),
        "<template><div /></template>\n<script>\nexport function caller() { helper(); }\n</script>\n",
    )
    .unwrap();
    let first = sync(root.path(), "Card.vue");
    assert_eq!(first["committed"], true, "{first}");
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='helper.ts'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM unresolved_refs WHERE language='vue' AND reference_name='helper' AND reference_kind='calls' AND status='pending'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM nodes WHERE file_path='Card.vue' AND id LIKE 'native:framework:%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            >= 1
    );

    fs::write(
        root.path().join("Card.vue"),
        "<template><div><span /></div></template>\n<script>\nexport function caller() { helper(); }\n</script>\n",
    )
    .unwrap();
    let refreshed =
        sync_file_forced(root.path(), "Card.vue", &OperationContext::default()).unwrap();
    assert_eq!(refreshed["committed"], true, "{refreshed}");
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM nodes WHERE file_path='Card.vue' AND kind='component' AND id LIKE 'native:framework:%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );

    fs::remove_file(root.path().join("Card.vue")).unwrap();
    let deleted = sync(root.path(), "Card.vue");
    assert_eq!(deleted["deleted"], true, "{deleted}");
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM nodes WHERE file_path='Card.vue' AND id LIKE 'native:framework:%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}
