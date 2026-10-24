use std::{
    ffi::OsString,
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

#[test]
fn public_api_returns_typed_requests_without_executing_them() {
    let parse_args: fn(Vec<OsString>) -> Result<Command, String> = parse;
    let needs_root: fn(&[OsString]) -> bool = needs_workspace_root;

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
fn cli_core_and_input_adapter_stay_outside_host_and_infrastructure() {
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
                line.is_empty() || line.starts_with('#'),
                "the parser should remain dependency-free; found `{line}` in {section}"
            );
        }
    }

    let mut all_files = Vec::new();
    rust_files(&root.join("src"), &mut all_files);
    assert!(!all_files.is_empty(), "CLI source is missing");
    for path in all_files {
        let source = fs::read_to_string(&path).unwrap();
        for forbidden in ["tokio::", "clap::", "harness_application::", "agent_harness::"] {
            assert!(
                !source.contains(forbidden),
                "{} imports host/runtime dependency `{forbidden}`",
                path.strip_prefix(root).unwrap().display()
            );
        }
    }

    for (layer, relative, forbidden) in [
        (
            "domain",
            "src/domain",
            &["crate::application", "crate::adapters", "crate::infrastructure", "crate::composition", "std::env", "std::fs", "std::process"][..],
        ),
        (
            "application",
            "src/application",
            &["crate::adapters", "crate::infrastructure", "crate::composition", "std::env", "std::fs", "std::process"][..],
        ),
        (
            "input adapter",
            "src/adapters",
            &["crate::infrastructure", "crate::composition"][..],
        ),
        (
            "infrastructure",
            "src/infrastructure",
            &["crate::composition"][..],
        ),
    ] {
        let path = root.join(relative);
        let mut files = Vec::new();
        rust_files(&path, &mut files);
        assert!(!files.is_empty(), "CLI {layer} source is missing");
        for path in files {
            let source = fs::read_to_string(&path).unwrap();
            for forbidden in forbidden {
                assert!(
                    !source.contains(forbidden),
                    "{} imports inward or host concern `{forbidden}`",
                    path.strip_prefix(root).unwrap().display()
                );
            }
        }
    }
}
