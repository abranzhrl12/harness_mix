use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn rust_files(root: &Path, output: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            output.push(path);
        }
    }
}

fn contains_inline_test(items: &[syn::Item]) -> bool {
    items.iter().any(|item| match item {
        syn::Item::Fn(function) => function.attrs.iter().any(|attribute| {
            attribute
                .path()
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "test")
        }),
        syn::Item::Mod(module) => module
            .content
            .as_ref()
            .is_some_and(|(_, items)| contains_inline_test(items)),
        _ => false,
    })
}

#[test]
fn query_presenters_do_not_read_sources_or_databases() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters");
    for relative in ["search/presentation.rs", "explore/presentation.rs"] {
        let source = fs::read_to_string(root.join(relative)).unwrap();
        for forbidden in [
            "Project",
            "OperationContext",
            "rusqlite",
            "std::fs",
            "tokio::fs",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} depends on I/O `{forbidden}`; pass observed data to the presenter"
            );
        }
    }
}

#[test]
fn transport_keeps_test_bodies_external_and_diagnostic_policy_out_of_native_adapters() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let transport = root.join("src/transport.rs");
    let mut files = vec![transport.clone()];
    rust_files(&root.join("src/transport"), &mut files);
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy();
        if name == "tests.rs" || name.ends_with("_tests.rs") {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        let parsed = syn::parse_file(&source).unwrap();
        assert!(
            !contains_inline_test(&parsed.items),
            "{} has inline tests",
            path.display()
        );
    }
    let source = fs::read_to_string(transport).unwrap();
    assert!(
        !source.contains("adapters::doctor::"),
        "protocol diagnostics must share domain policy instead of calling a native adapter"
    );
}

#[test]
fn service_tests_stay_in_external_modules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters");
    let mut files = vec![
        root.join("verify.rs"),
        root.join("search.rs"),
        root.join("explore.rs"),
        root.join("tasks.rs"),
        root.join("index_store.rs"),
        root.join("process.rs"),
        root.join("sql.rs"),
        root.join("git.rs"),
        root.join("index_watch.rs"),
        root.join("postgresql.rs"),
        root.join("index_extract.rs"),
        root.join("ignore.rs"),
        root.join("tasks_store.rs"),
        root.join("process_status.rs"),
    ];
    for directory in [
        "verification",
        "search",
        "explore",
        "tasks",
        "index_store",
        "process",
        "sql",
        "git",
        "index_watch",
        "postgresql",
        "index_extract",
        "ignore",
        "tasks_store",
        "process_status",
    ] {
        rust_files(&root.join(directory), &mut files);
    }
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy();
        if name == "tests.rs"
            || name.ends_with("_tests.rs")
            || path
                .strip_prefix(&root)
                .unwrap()
                .components()
                .any(|component| component.as_os_str() == "tests")
        {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        let parsed = syn::parse_file(&source).unwrap();
        assert!(
            !contains_inline_test(&parsed.items),
            "{} contains inline tests; keep test bodies in separate files",
            path.strip_prefix(&root).unwrap().display()
        );
    }
}

#[test]
fn application_logging_tests_stay_external() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/application/src");
    let mut files = vec![root.join("logging.rs")];
    rust_files(&root.join("logging"), &mut files);
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy();
        if name == "tests.rs" || name.ends_with("_tests.rs") {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        let parsed = syn::parse_file(&source).unwrap();
        assert!(
            !contains_inline_test(&parsed.items),
            "{} contains inline tests",
            path.display()
        );
    }
}

#[test]
fn logging_sanitization_and_job_snapshot_validation_stay_pure() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for relative in [
        "crates/application/src/logging/sanitize.rs",
        "src/adapters/tasks_store/snapshot.rs",
    ] {
        let source = fs::read_to_string(root.join(relative)).unwrap();
        for forbidden in [
            "std::fs",
            "tokio::",
            "rusqlite",
            "Project",
            "OperationContext",
            "std::net",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} depends on infrastructure `{forbidden}`"
            );
        }
    }
}

#[test]
fn job_store_retains_resource_ownership_and_drop_order() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters/tasks_store.rs"),
    )
    .unwrap();
    let parsed = syn::parse_file(&source).unwrap();
    let store = parsed
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "Store" => Some(item),
            _ => None,
        })
        .expect("job store resource owner");
    let names = store
        .fields
        .iter()
        .map(|field| {
            assert!(
                matches!(field.vis, syn::Visibility::Inherited),
                "store handles must stay private"
            );
            field.ident.as_ref().unwrap().to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        ["db", "_files", "_writer", "_directories", "directory"],
        "SQLite must close before its file pins and writer lock are released"
    );
}

#[test]
fn extraction_decoder_stays_independent_of_workspace_io() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters/index_extract");
    for relative in ["decode.rs", "csharp.rs", "razor.rs"] {
        let source = fs::read_to_string(root.join(relative)).unwrap();
        for forbidden in [
            "Project",
            "std::fs",
            "tokio::fs",
            "rusqlite",
            "crate::transport",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} depends on workspace infrastructure `{forbidden}`"
            );
        }
    }
}

#[test]
fn ignore_policy_transformation_stays_pure() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters/ignore/policy.rs"),
    )
    .unwrap();
    for forbidden in [
        "Project",
        "std::fs",
        "tokio::fs",
        "OperationContext",
        "tempfile",
    ] {
        assert!(
            !source.contains(forbidden),
            "ignore policy depends on infrastructure `{forbidden}`"
        );
    }
}

#[test]
fn workspace_is_a_composition_bound_instead_of_a_monolithic_interface() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source =
        fs::read_to_string(root.join("crates/application/src/ports/workspace/mod.rs")).unwrap();
    let parsed = syn::parse_file(&source).unwrap();
    let workspace = parsed
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Trait(item) if item.ident == "Workspace" => Some(item),
            _ => None,
        })
        .expect("workspace composition contract");
    assert!(
        workspace.items.is_empty(),
        "add methods to capability ports, not Workspace"
    );
    assert!(
        workspace.supertraits.len() > 1,
        "compose independent capabilities"
    );
}

#[test]
fn portable_layers_cannot_gain_unreviewed_infrastructure_dependencies() {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--offline",
            "--locked",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let packages = metadata["packages"].as_array().unwrap();
    for (name, allowed) in [
        (
            "harness-domain",
            &["serde", "serde_json", "sha2", "uuid", "schemars"][..],
        ),
        ("harness-application", &["harness-domain", "serde_json"][..]),
    ] {
        let package = packages
            .iter()
            .find(|package| package["name"] == name)
            .unwrap();
        for dependency in package["dependencies"].as_array().unwrap() {
            if dependency["kind"] == "dev" {
                continue;
            }
            let dependency = dependency["name"].as_str().unwrap();
            assert!(
                allowed.contains(&dependency),
                "{name} depends on {dependency}; infrastructure belongs in native adapters, not portable layers"
            );
        }
    }
}

#[test]
fn language_parsers_stay_independent_of_projects_transport_and_filesystem() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters/parsers");
    let mut files = Vec::new();
    rust_files(&root, &mut files);

    assert!(!files.is_empty(), "parser source tree is missing");
    let external_tests = files
        .iter()
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("_tests.rs")
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut production_sources = Vec::new();
    for path in files.iter().filter(|path| {
        !path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("_tests.rs")
    }) {
        let source = fs::read_to_string(path).unwrap();
        let relative = path.strip_prefix(&root).unwrap().display();
        production_sources.push(source.clone());
        for forbidden in [
            "crate::adapters::project::Project",
            "adapters::project",
            "project::Project",
            "Workspace",
            "crate::transport",
            "std::fs",
            "tokio::fs",
            "File::open",
            "OpenOptions",
            "read_to_string(",
            "read_dir(",
            "canonicalize(",
            "rusqlite",
            "Connection",
        ] {
            assert!(
                !source.contains(forbidden),
                "parser source {relative} refers to forbidden infrastructure `{forbidden}`"
            );
        }

        assert!(
            !source.contains("#[test]") && !source.contains("mod tests {"),
            "tests in parser source {relative} must live in a separate *_tests.rs module"
        );
    }

    for path in external_tests {
        let file_name = path.file_name().unwrap().to_string_lossy();
        let module_name = path.file_stem().unwrap().to_string_lossy();
        let path_declaration = format!("= \"{file_name}\"");
        let module_declaration = format!("mod {module_name};");
        assert!(
            production_sources.iter().any(|source| {
                source.contains(&path_declaration) || source.contains(&module_declaration)
            }),
            "external parser tests {} must be declared by a production module",
            path.strip_prefix(&root).unwrap().display()
        );
    }
}

#[test]
fn source_analysis_domain_does_not_depend_on_parser_engines_or_io() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/domain/src/parsing");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(!files.is_empty(), "source analysis domain is missing");
    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        for forbidden in [
            "oxc_",
            "tree_sitter",
            "syn::",
            "proc_macro2",
            "rusqlite",
            "std::fs",
            "tokio::",
            "crate::adapters",
            "harness_application",
            "rmcp",
        ] {
            assert!(
                !source.contains(forbidden),
                "domain source {} depends on infrastructure `{forbidden}`",
                path.strip_prefix(&root).unwrap().display()
            );
        }
        assert!(
            !source.contains("#[test]") && !source.contains("mod tests {"),
            "domain parsing tests must live in crates/domain/tests"
        );
    }
}
