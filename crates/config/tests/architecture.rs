use std::{fs, path::Path};

#[test]
fn configuration_does_not_construct_or_depend_on_services() {
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
            "configuration must not depend on {dependency}"
        );
    }
}

#[test]
fn library_root_is_a_minimal_public_facade() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join("src/lib.rs")).unwrap();
    let declarations = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"));

    for declaration in declarations {
        assert!(
            (declaration.starts_with("mod ") || declaration.starts_with("pub use "))
                && declaration.ends_with(';')
                || declaration == "#[cfg(test)]",
            "lib.rs should contain declarations and re-exports, not implementation: {declaration}"
        );
    }

    for module in ["environment.rs", "runtime.rs", "tests.rs"] {
        assert!(
            root.join("src").join(module).is_file(),
            "expected config module src/{module}"
        );
    }
}

#[test]
fn runtime_validation_does_not_read_process_state_or_files() {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runtime.rs")).unwrap();
    for operation in ["env::var(", "env::var_os(", "std::fs", "std::process"] {
        assert!(
            !source.contains(operation),
            "runtime validation must use supplied values, not {operation}"
        );
    }
}
