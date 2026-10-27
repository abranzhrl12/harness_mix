#![cfg(windows)]
use agent_harness::adapters::index_store::{initialize, sync_changed_paths};
use harness_domain::execution::OperationContext;
use std::{collections::BTreeSet, fs};

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::create_dir(root.path().join("blocked")).unwrap();
    fs::write(root.path().join("blocked/.gitignore"), "!resurrected.rs\n").unwrap();
    fs::create_dir(root.path().join(".git")).unwrap();
    fs::create_dir(root.path().join(".git/info")).unwrap();
    fs::write(root.path().join(".git/info/exclude"), "src/git.rs\n").unwrap();
    fs::write(root.path().join(".gitignore"), "src/root.rs\nblocked/\n").unwrap();
    fs::write(root.path().join("src/.gitignore"), "local.rs\n").unwrap();
    fs::write(
        root.path().join("codegraph.json"),
        r#"{"include":["src/root.rs"],"exclude":["src/excluded.rs"]}"#,
    )
    .unwrap();
    root
}

#[test]
fn probe_per_path_behavior() {
    let context = OperationContext::default();
    let names = [
        "blocked/resurrected.rs",
        "src/root.rs",
        "src/local.rs",
        "src/git.rs",
        "src/keep.rs",
        "src/excluded.rs",
    ];
    for name in names {
        let root = fixture();
        fs::write(root.path().join(&name.replace('/', r"\")), "pub fn sample() {}\n").unwrap();
        // Write sibling files so .gitignore files exist as in the real fixture
        fs::write(
            root.path().join("blocked/resurrected.rs"),
            "pub fn sample() {}\n",
        )
        .unwrap();
        fs::write(root.path().join("src/root.rs"), "pub fn sample() {}\n").unwrap();
        fs::write(root.path().join("src/local.rs"), "pub fn sample() {}\n").unwrap();
        fs::write(root.path().join("src/git.rs"), "pub fn sample() {}\n").unwrap();
        fs::write(root.path().join("src/keep.rs"), "pub fn sample() {}\n").unwrap();
        fs::write(root.path().join("src/excluded.rs"), "pub fn sample() {}\n").unwrap();
        let result = sync_changed_paths(root.path(), &BTreeSet::from([name.to_string()]), &context)
            .unwrap();
        println!(
            "PATH {name}: processed={} committed={} skipped={}",
            result["filesProcessed"],
            result["changedFilesCommitted"],
            result["skippedFiles"]
        );
    }
}
