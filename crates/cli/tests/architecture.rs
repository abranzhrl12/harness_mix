use std::{
    fs,
    path::{Path, PathBuf},
};

use harness_cli::{Command, Root, needs_workspace_root, parse};

fn rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn assert_source_avoids(directory: &Path, forbidden: &[&str], layer: &str) {
    let mut files = Vec::new();
    rust_files(directory, &mut files);
    assert!(!files.is_empty(), "CLI {layer} source is missing");
    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        for pattern in forbidden {
            assert!(
                !source.contains(pattern),
                "{} crosses the {layer} boundary with `{pattern}`",
                path.display()
            );
        }
    }
}

#[test]
fn public_api_returns_typed_requests_without_executing_them() {
    let parse_args: fn(Vec<std::ffi::OsString>) -> Result<Command, String> = parse;
    let needs_root: fn(&[std::ffi::OsString]) -> bool = needs_workspace_root;

    assert!(!needs_root(&[]));
    assert!(matches!(
        parse_args(Vec::new()),
        Ok(Command::Serve {
            root: Root::DetectOrCurrent,
            workspace_config: None,
            persist_logs: false,
        })
    ));
}

#[test]
fn parser_core_does_not_depend_on_execution_or_outer_cli_modules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    assert_source_avoids(
        &root.join("domain"),
        &[
            "crate::application",
            "crate::adapters",
            "crate::infrastructure",
            "crate::composition",
            "crate::commands",
            "crate::terminal",
            "std::env",
            "std::fs",
            "std::process",
            "tokio::",
            "harness_application::",
            "agent_harness::",
        ],
        "parser domain",
    );
    assert_source_avoids(
        &root.join("application"),
        &[
            "crate::adapters",
            "crate::infrastructure",
            "crate::composition",
            "crate::commands",
            "crate::terminal",
            "std::env",
            "std::fs",
            "std::process",
            "tokio::",
            "harness_application::",
            "agent_harness::",
        ],
        "parser application",
    );
    assert_source_avoids(
        &root.join("adapters/input"),
        &[
            "crate::infrastructure",
            "crate::composition",
            "crate::commands",
            "crate::terminal",
            "std::env",
            "std::fs",
            "std::process",
            "tokio::",
            "harness_application::",
            "agent_harness::",
        ],
        "argument parser",
    );
}

#[test]
fn cli_crate_never_depends_on_the_host_crate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let mut section = "";
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            section = line;
            continue;
        }
        if matches!(section, "[dependencies]" | "[dev-dependencies]") {
            assert!(
                !line.contains("agent-harness") && !line.contains("agent_harness"),
                "the CLI crate must not depend on its host package: `{line}`"
            );
        }
    }

    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        assert!(
            !source.contains("agent_harness"),
            "{} imports the host package; inject host behavior through a port",
            path.strip_prefix(root).unwrap().display()
        );
    }
}
