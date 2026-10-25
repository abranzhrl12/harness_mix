use std::{fs, path::Path};

#[test]
fn skills_do_not_depend_on_entry_points_or_server_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for dependency in [
        "harness-application",
        "harness-cli",
        "harness-mcp",
        "agent-harness",
        "tokio",
        "rmcp",
    ] {
        assert!(
            !manifest.contains(dependency),
            "skills must not depend on {dependency}"
        );
    }
}

#[test]
fn facade_keeps_filesystem_primitives_out_of_the_general_api() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let facade = fs::read_to_string(root.join("src/lib.rs")).unwrap();
    for implementation_module in [
        "mod catalog;",
        "mod lock;",
        "mod manifest;",
        "mod preferences;",
        "mod report;",
        "mod storage;",
        "mod sync;",
    ] {
        assert!(facade.contains(implementation_module));
    }
    assert!(facade.contains("pub mod project_support"));
    assert!(!facade.contains("pub use sync::{checked_root"));
    assert!(!facade.contains("pub use sync::{read_safe_file"));
    for module in [
        "catalog",
        "lock",
        "manifest",
        "preferences",
        "report",
        "storage",
        "sync",
    ] {
        assert!(!facade.contains(&format!("pub mod {module};")));
    }
    for line in facade
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
    {
        assert!(
            (line.starts_with("mod ")
                || line.starts_with("pub mod ")
                || line.starts_with("pub use "))
                && line.ends_with(';'),
            "lib.rs must only declare modules and exports: {line}"
        );
    }
}
