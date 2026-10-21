use agent_harness::adapters::index_sync::plan_workspace;
use harness_domain::execution::OperationContext;
use std::fs;
#[path = "support/codegraph.rs"]
mod graph_fixture;

#[test]
fn vue_and_html_are_discovered_previewed_and_ignored_without_index_writes() {
    let root = tempfile::tempdir().unwrap();
    drop(graph_fixture::fixture(root.path()));
    fs::write(root.path().join("App.vue"), "<template><p>Hello</p></template><script setup lang=\"ts\">const name: string = 'Vue';</script>").unwrap();
    fs::write(
        root.path().join("app.component.html"),
        "<button (click)=\"save()\">{{ title }}</button>",
    )
    .unwrap();
    fs::write(root.path().join("hidden.vue"), "<template/>").unwrap();
    fs::write(root.path().join(".gitignore"), "hidden.vue\n").unwrap();
    let database = root.path().join(".codegraph/codegraph.db");
    let before = fs::read(&database).unwrap();
    let result = plan_workspace(root.path(), &OperationContext::default()).unwrap();
    assert_eq!(
        result["plan"]["added"],
        serde_json::json!(["App.vue", "app.component.html"]),
        "{result}"
    );
    let previews = result["extractionPreview"]["files"].as_array().unwrap();
    assert_eq!(previews.len(), 2, "{result}");
    for file in previews {
        assert_eq!(file["status"], "extracted_not_published", "{file}");
        assert!(file["extractionCoverage"].is_object(), "{file}");
    }
    assert_eq!(before, fs::read(database).unwrap());
    assert_eq!(result["extractionPreview"]["published"], false);
}
