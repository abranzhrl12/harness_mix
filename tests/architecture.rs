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
fn indexing_application_contract_and_policy_stay_portable() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![
        root.join("crates/application/src/ports/indexing.rs"),
        root.join("crates/application/src/indexing.rs"),
    ];
    rust_files(&root.join("crates/application/src/indexing"), &mut files);
    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        for forbidden in [
            "rusqlite",
            "crate::adapters",
            "std::fs",
            "tokio::",
            "codegraph_native",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} depends on infrastructure {forbidden}",
                path.display()
            );
        }
    }
}

#[test]
fn indexing_facade_delegates_lifecycles_instead_of_implementing_them() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters/index_store.rs"),
    )
    .unwrap();
    for forbidden in [
        "for (position, path)",
        "let mut processed",
        "tx.commit()",
        "tx.rollback()",
        "parallel::prepare(",
    ] {
        assert!(
            !source.contains(forbidden),
            "indexing facade still owns lifecycle {forbidden}"
        );
    }
    for use_case in [
        "indexing::workspace::sync_workspace",
        "indexing::changed_paths::sync_changed_paths",
        "indexing::file::sync_file",
    ] {
        assert!(
            source.contains(use_case),
            "missing application entry point {use_case}"
        );
    }
}

#[test]
fn mcp_keeps_test_bodies_external_and_diagnostic_policy_out_of_native_adapters() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mcp = root.join("crates/mcp/src");
    let mut files = Vec::new();
    rust_files(&mcp, &mut files);
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
        assert!(
            !source.contains("adapters::doctor::"),
            "protocol diagnostics must share domain policy instead of calling a native adapter"
        );
    }
}

#[test]
fn service_tests_stay_in_external_modules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters");
    let mut files = vec![
        root.join("verify.rs"),
        root.join("search.rs"),
        root.join("explore.rs"),
        root.join("tasks.rs"),
        root.join("task_execution.rs"),
        root.join("task_pinning.rs"),
        root.join("index_store.rs"),
        root.join("process.rs"),
        root.join("sql.rs"),
        root.join("git.rs"),
        root.join("index_watch.rs"),
        root.join("postgresql.rs"),
        root.join("index_extract.rs"),
        root.join("ignore.rs"),
        root.join("process_status.rs"),
        root.join("sessions.rs"),
        root.join("symbol_graph.rs"),
        root.join("index_imports.rs"),
        root.join("index_semantic.rs"),
    ];
    for directory in [
        "verification",
        "search",
        "explore",
        "index_store",
        "process",
        "sql",
        "git",
        "index_watch",
        "postgresql",
        "index_extract",
        "ignore",
        "process_status",
        "sessions",
        "symbol_graph",
        "index_imports",
        "index_semantic",
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
fn symbol_graph_classification_and_model_stay_pure() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters/symbol_graph");
    for relative in ["classification.rs", "model.rs"] {
        let source = fs::read_to_string(root.join(relative)).unwrap();
        for forbidden in [
            "Project",
            "rusqlite",
            "std::fs",
            "tokio::",
            "OperationContext",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} depends on infrastructure `{forbidden}`"
            );
        }
    }
}

#[test]
fn domain_sql_tests_stay_external_and_policy_stays_portable() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/domain/src");
    let mut files = vec![root.join("sql.rs")];
    rust_files(&root.join("sql"), &mut files);
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
        for forbidden in [
            "rusqlite",
            "tokio::",
            "std::fs",
            "crate::adapters",
            "harness_application",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} depends on infrastructure `{forbidden}`",
                path.display()
            );
        }
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
        "crates/jobs/src/store/snapshot.rs",
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
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/jobs/src/store.rs"),
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
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy();
        if name == "preview.rs" || name == "tests.rs" || name.ends_with("_tests.rs") {
            continue;
        }
        let relative = path.strip_prefix(&root).unwrap().display();
        let source = fs::read_to_string(&path).unwrap();
        // Documentation may use "project" as a verb; inspect production lines.
        let production = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "Project",
            "std::fs",
            "tokio::fs",
            "rusqlite",
            "crate::transport",
        ] {
            assert!(
                !production.contains(forbidden),
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
            // The domain owns the pure, deterministic UTF-8/Windows-1252
            // source decoding policy; this codec is not an I/O dependency.
            &["serde", "serde_json", "sha2", "uuid", "schemars", "encoding_rs"][..],
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
fn browser_cdp_stays_in_an_optional_outward_adapter_crate() {
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
    fn package<'a>(packages: &'a [Value], name: &str) -> &'a Value {
        packages
            .iter()
            .find(|package| package["name"] == name)
            .unwrap_or_else(|| panic!("workspace package {name} is missing"))
    }
    fn dependency_names(package: &Value) -> Vec<String> {
        package["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|dependency| dependency["name"].as_str().map(str::to_owned))
            .collect::<Vec<_>>()
    }

    let root = package(packages, "agent-harness");
    let root_dependencies = dependency_names(root);
    assert!(
        !root_dependencies.iter().any(|name| name == "chromiumoxide"),
        "the executable should depend on the browser adapter, not CDP directly"
    );
    assert!(
        !root_dependencies.iter().any(|name| name == "url"),
        "CDP-only URL parsing belongs in the optional browser adapter"
    );
    let browser_dependency = root["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|dependency| dependency["name"] == "harness-browser")
        .expect("root composition depends on harness-browser");
    assert_eq!(browser_dependency["optional"], true);
    let browser_feature = root["features"]["browser-cdp"]
        .as_array()
        .expect("browser-cdp feature exists");
    assert!(
        browser_feature
            .iter()
            .any(|value| value == "dep:harness-browser")
    );
    assert!(
        root["features"]["default"]
            .as_array()
            .is_some_and(|features| features.iter().any(|value| value == "browser-cdp")),
        "normal builds enable browser support while --no-default-features can omit CDP"
    );

    let browser = package(packages, "harness-browser");
    let default_members = metadata["workspace_default_members"]
        .as_array()
        .expect("workspace default-members are reported by Cargo metadata");
    assert!(
        !default_members.iter().any(|id| id == &browser["id"]),
        "harness-browser is pulled through the root feature, not built as a default workspace member"
    );
    let browser_dependencies = dependency_names(browser);
    for dependency in ["chromiumoxide", "harness-application", "harness-domain"] {
        assert!(
            browser_dependencies.iter().any(|name| name == dependency),
            "harness-browser must depend on {dependency}"
        );
    }
    assert!(
        !browser_dependencies
            .iter()
            .any(|name| name == "agent-harness"),
        "the browser adapter cannot depend back on its composition root"
    );
    for layer in [
        package(packages, "harness-domain"),
        package(packages, "harness-application"),
    ] {
        let dependencies = dependency_names(layer);
        assert!(
            !dependencies.iter().any(|name| name == "harness-browser")
                && !dependencies.iter().any(|name| name == "chromiumoxide"),
            "portable layers cannot depend on the browser adapter or CDP"
        );
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

#[test]
fn publication_planning_and_models_stay_independent_of_io() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters/index_store/publication");
    for relative in ["model.rs", "plan.rs"] {
        let source = fs::read_to_string(root.join(relative)).unwrap();
        for forbidden in [
            "rusqlite",
            "std::fs",
            "tokio::",
            "Project",
            "Transaction",
            "index_extract::extract",
            "SystemTime::now",
            "storage::",
        ] {
            assert!(
                !source.contains(forbidden),
                "publication {relative} depends on I/O `{forbidden}`; pass observed data to planning"
            );
        }
    }
}

#[test]
fn mcp_job_handlers_depend_on_application_ports_instead_of_native_scheduler() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("crates/mcp/src"), &mut files);
    for path in files {
        let file_name = path.file_name().unwrap().to_string_lossy();
        if file_name == "tests.rs" || file_name.ends_with("_tests.rs") {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        for forbidden in [
            "NativeApplication",
            "TaskManager",
            "adapters::tasks",
            "broadcast::",
            "RecvError::",
            "std::env",
            "env::var",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} depends on native job infrastructure `{forbidden}`",
                path.display()
            );
        }
    }
}

#[test]
fn sql_server_execution_does_not_choose_or_call_other_engines() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join("src/adapters/sql/operations.rs")).unwrap();
    for forbidden in [
        "select_engine",
        "adapters::postgresql",
        "SqlEngine::Postgresql",
    ] {
        assert!(
            !source.contains(forbidden),
            "SQL Server execution depends on routing `{forbidden}`; select the backend in the SQL facade"
        );
    }
}

#[test]
fn sql_presentation_receives_data_without_driver_or_workspace_access() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join("src/adapters/sql/presentation.rs")).unwrap();
    for forbidden in [
        "tiberius",
        "tokio::",
        "std::fs",
        "Project",
        "block_on_safe",
        "connection::",
        "operations::",
        "load_sql_config",
        "select_engine",
    ] {
        assert!(
            !source.contains(forbidden),
            "SQL presentation depends on infrastructure `{forbidden}`; pass observed catalog data"
        );
    }
    let parsed = syn::parse_file(&source).unwrap();
    assert!(
        !contains_inline_test(&parsed.items),
        "keep SQL presentation tests in an external module"
    );
}

#[test]
fn native_workspace_capabilities_delegate_to_services() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters/workspace");
    for (relative, forbidden) in [
        (
            "source_editing.rs",
            &["edit::", "move_file::", "try_lock("][..],
        ),
        (
            "source_queries.rs",
            &["explore::", "search::", "symbols::", "edit_lock"][..],
        ),
        (
            "session_persistence.rs",
            &["persist_snapshot(", "handle_handover("][..],
        ),
    ] {
        let source = fs::read_to_string(root.join(relative)).unwrap();
        for forbidden in forbidden {
            assert!(
                !source.contains(forbidden),
                "workspace facade {relative} implements `{forbidden}` directly instead of delegating to its injected service"
            );
        }
    }
    let mut files = Vec::new();
    rust_files(&root.join("services"), &mut files);
    assert!(!files.is_empty(), "workspace services are missing");
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy();
        if name == "tests.rs" || name.ends_with("_tests.rs") {
            continue;
        }
        let parsed = syn::parse_file(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(
            !contains_inline_test(&parsed.items),
            "{} has inline tests; keep workspace service tests in external modules",
            path.display()
        );
    }
}

#[test]
fn application_dispatch_delegates_to_typed_capability_use_cases() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/application/src");
    let dispatch = fs::read_to_string(root.join("dispatch.rs")).unwrap();
    for forbidden in [
        ".sessions",
        "refresh_checkpoint(",
        "result_ref.take()",
        "git_session_changes(",
    ] {
        assert!(
            !dispatch.contains(forbidden),
            "dispatch implements `{forbidden}`; keep capability policy in its use-case module"
        );
    }
    let mut files = Vec::new();
    rust_files(&root.join("dispatch"), &mut files);
    assert!(!files.is_empty(), "capability use cases are missing");
    for path in files {
        let file_name = path.file_name().unwrap().to_string_lossy();
        if file_name == "tests.rs" || file_name.ends_with("_tests.rs") {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        for forbidden in [
            "contracts::Request",
            "unreachable!",
            "std::fs",
            "tokio::",
            "rmcp",
            "crate::adapters",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} depends on `{forbidden}`; use typed arguments and application ports",
                path.display()
            );
        }
        assert!(
            !contains_inline_test(&syn::parse_file(&source).unwrap().items),
            "keep capability use-case tests in external modules"
        );
    }
}

#[test]
fn portable_layers_do_not_import_tokio_or_browser_cdp_types() {
    use syn::visit::Visit;

    #[derive(Default)]
    struct ForbiddenArchitecturePaths {
        layer: &'static str,
        paths: Vec<String>,
    }

    fn record_if_forbidden(layer: &str, segments: &[String], output: &mut Vec<String>) {
        let runtime_or_cdp = segments
            .first()
            .is_some_and(|root| root == "tokio" || root == "chromiumoxide" || root == "cdp");
        let inward_layer = segments.windows(2).any(|pair| {
            pair[0] == "crate"
                && (pair[1] == "infrastructure"
                    || pair[1] == "composition"
                    || (layer == "domain" && pair[1] == "application"))
        });
        let application_in_domain = layer == "domain"
            && segments
                .first()
                .is_some_and(|root| root == "harness_application");
        if runtime_or_cdp || inward_layer || application_in_domain {
            output.push(segments.join("::"));
        }
    }

    fn collect_use_tree(
        tree: &syn::UseTree,
        prefix: &mut Vec<String>,
        imports: &mut Vec<Vec<String>>,
    ) {
        match tree {
            syn::UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                collect_use_tree(&path.tree, prefix, imports);
                prefix.pop();
            }
            syn::UseTree::Name(name) => {
                let mut path = prefix.clone();
                path.push(name.ident.to_string());
                imports.push(path);
            }
            syn::UseTree::Rename(rename) => {
                let mut path = prefix.clone();
                path.push(rename.ident.to_string());
                imports.push(path);
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    collect_use_tree(item, prefix, imports);
                }
            }
            syn::UseTree::Glob(_) => imports.push(prefix.clone()),
        }
    }

    impl<'ast> Visit<'ast> for ForbiddenArchitecturePaths {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            let segments = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            record_if_forbidden(self.layer, &segments, &mut self.paths);
            syn::visit::visit_path(self, path);
        }

        fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
            let mut imports = Vec::new();
            collect_use_tree(&item.tree, &mut Vec::new(), &mut imports);
            for import in imports {
                record_if_forbidden(self.layer, &import, &mut self.paths);
            }
            syn::visit::visit_item_use(self, item);
        }
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture = syn::parse_file(
        "use std::sync::{Arc, Mutex};\nuse crate::{infrastructure::Backend, composition::Service};\nuse chromiumoxide::cdp::Target;\nuse tokio::runtime::Runtime;",
    )
    .unwrap();
    let mut fixture_visitor = ForbiddenArchitecturePaths {
        layer: "application",
        ..Default::default()
    };
    fixture_visitor.visit_file(&fixture);
    assert!(
        fixture_visitor
            .paths
            .iter()
            .any(|path| path == "crate::infrastructure::Backend")
    );
    assert!(
        fixture_visitor
            .paths
            .iter()
            .any(|path| path == "crate::composition::Service")
    );
    assert!(
        fixture_visitor
            .paths
            .iter()
            .any(|path| path == "chromiumoxide::cdp::Target")
    );
    assert!(
        fixture_visitor
            .paths
            .iter()
            .any(|path| path == "tokio::runtime::Runtime")
    );
    assert!(
        !fixture_visitor
            .paths
            .iter()
            .any(|path| path.starts_with("std::sync"))
    );

    for (layer, path) in [
        ("domain", "crates/domain/src"),
        ("application", "crates/application/src"),
        ("domain", "crates/browser/src/domain"),
        ("application", "crates/browser/src/application"),
    ] {
        let mut files = Vec::new();
        let layer_root = root.join(path);
        assert!(layer_root.is_dir(), "portable layer {path} is missing");
        rust_files(&layer_root, &mut files);
        assert!(
            !files.is_empty(),
            "portable layer {path} has no Rust sources"
        );
        for path in files {
            let source = fs::read_to_string(&path).unwrap();
            let parsed = syn::parse_file(&source).unwrap();
            let mut visitor = ForbiddenArchitecturePaths {
                layer,
                ..Default::default()
            };
            visitor.visit_file(&parsed);
            assert!(
                visitor.paths.is_empty(),
                "{} imports inward/runtime infrastructure {:?}; keep CDP, Tokio, composition, and app policy outside portable domain",
                path.strip_prefix(root).unwrap().display(),
                visitor.paths
            );
        }
    }
}
