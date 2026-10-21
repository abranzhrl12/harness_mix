#![cfg(windows)]
use agent_harness::adapters::{self, index_store::sync_file};
use harness_domain::{contracts::Request, execution::OperationContext};
use rusqlite::Connection;
use serde_json::json;
use std::fs;

#[test]
fn published_vue_and_angular_components_are_searchable_as_certified_components() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".codegraph")).unwrap();
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0)",
        [],
    )
    .unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    drop(db);
    fs::write(
        root.path().join("Card.vue"),
        "<template><p>Vue</p></template>",
    )
    .unwrap();
    fs::write(root.path().join("app.ts"), "import { Component } from '@angular/core'; @Component({template: '<p>Angular</p>'}) export class App {}").unwrap();
    for path in ["Card.vue", "app.ts"] {
        let published = sync_file(root.path(), path, &OperationContext::default()).unwrap();
        assert_eq!(published["committed"], true, "{published}");
    }
    let app = adapters::application(root.path()).unwrap();
    let request =
        serde_json::from_value(json!({"kind":"syntax","query":"component","scope":".","limit":20}))
            .unwrap();
    let result = app.execute(Request::Search(request));
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["discovery"], "index", "{result}");
    let matches = result["data"]["matches"].as_array().unwrap();
    for name in ["Card", "App"] {
        assert!(
            matches
                .iter()
                .any(|item| item["name"] == name && item["freshness"] == "fresh"),
            "{result}"
        );
    }
}
