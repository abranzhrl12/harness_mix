use std::{
    fs,
    path::{Path, PathBuf},
};

fn sources(root: &Path, paths: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, paths);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            paths.push(path);
        }
    }
}

#[test]
fn job_engine_does_not_depend_on_entry_points_or_native_verifiers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for dependency in [
        "agent-harness",
        "harness-cli",
        "harness-mcp",
        "harness-browser",
        "rmcp",
    ] {
        assert!(
            !manifest.contains(dependency),
            "jobs must not depend on {dependency}"
        );
    }
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        for forbidden in [
            "crate::adapters",
            "agent_harness::",
            "NativeProcessRunner",
            "verify::background_plan",
            "verify::angular",
            "run_typescript_syntax",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} couples jobs to {forbidden}",
                path.display()
            );
        }
    }
}

#[test]
fn library_root_is_only_a_facade() {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs")).unwrap();
    for line in source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
    {
        assert!(
            ((line.starts_with("mod ") || line.starts_with("pub use ")) && line.ends_with(';'))
                || line == "#[cfg(test)]",
            "lib.rs must declare modules and exports only: {line}"
        );
    }
}
