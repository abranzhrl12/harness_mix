use harness_oxc_evaluation::{
    imported_references,
    modules::{self, Snapshot},
};
use oxc_resolver::{TsconfigOptions, TsconfigReferences};
use std::path::PathBuf;

fn root() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\harness-oxc-synthetic")
    } else {
        PathBuf::from("/harness-oxc-synthetic")
    }
}

#[test]
fn named_alias_resolves_to_imported_symbol() {
    let source = "import {run as execute} from './lib'; export function caller() { execute(); }";
    assert_eq!(
        imported_references(source, "app.ts", "execute").unwrap(),
        [source.rfind("execute()").unwrap() as u32]
    );
}

#[test]
fn shadowed_parameter_does_not_reference_import() {
    let source = "import {run as execute} from './lib'; export function caller(execute: () => void) { execute(); }";
    assert!(
        imported_references(source, "app.ts", "execute")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn mixed_scopes_preserve_real_import_reference_unlike_current_guard() {
    let source = "import {run as execute} from './lib'; function local(execute: () => void) { execute(); } export function caller() { execute(); }";
    assert!(!codegraph_native::unshadowed_import_calls(
        source,
        "typescript",
        "execute"
    ));
    assert_eq!(
        imported_references(source, "app.ts", "execute").unwrap(),
        [source.rfind("execute()").unwrap() as u32]
    );
}

#[test]
fn block_local_binding_does_not_hide_import_outside_block() {
    let source = "import {run as execute} from './lib'; export function caller() { {const execute = () => {}; execute();} execute(); }";
    assert!(!codegraph_native::unshadowed_import_calls(
        source,
        "typescript",
        "execute"
    ));
    assert_eq!(
        imported_references(source, "app.ts", "execute").unwrap(),
        [source.rfind("execute()").unwrap() as u32]
    );
}

#[test]
fn imported_value_reference_and_tsx_are_supported() {
    let source = "import {run as execute} from './lib'; export function View() { return <button onClick={execute}>😀</button>; }";
    assert!(!codegraph_native::unshadowed_import_calls(
        source, "tsx", "execute"
    ));
    assert_eq!(
        imported_references(source, "app.tsx", "execute").unwrap(),
        [source.rfind("execute}").unwrap() as u32]
    );
}

#[test]
fn invalid_syntax_and_oversize_are_rejected() {
    assert!(imported_references("function {", "app.ts", "x").is_err());
    assert!(imported_references(&" ".repeat(1024 * 1024 + 1), "app.ts", "x").is_err());
}

#[test]
fn resolves_relative_extensionless_and_directory_index() {
    let base = root();
    let resolver = modules::resolver(
        Snapshot::new(base.clone(), &[("lib.ts", ""), ("nested/index.ts", "")]),
        modules::options(),
    );
    assert_eq!(
        resolver.resolve(&base, "./lib").unwrap().path(),
        base.join("lib.ts")
    );
    assert_eq!(
        resolver.resolve(&base, "./nested").unwrap().path(),
        base.join("nested/index.ts")
    );
}

#[test]
fn emitted_js_specifier_resolves_to_typescript_source() {
    let base = root();
    let resolver = modules::resolver(
        Snapshot::new(base.clone(), &[("lib.ts", "")]),
        modules::options(),
    );
    assert_eq!(
        resolver.resolve(&base, "./lib.js").unwrap().path(),
        base.join("lib.ts")
    );
}

#[test]
fn tsconfig_paths_resolve_without_node_or_real_filesystem() {
    let base = root();
    let mut options = modules::options();
    options.tsconfig = Some(TsconfigOptions {
        config_file: base.join("tsconfig.json"),
        references: TsconfigReferences::Disabled,
    });
    let resolver = modules::resolver(
        Snapshot::new(
            base.clone(),
            &[
                (
                    "tsconfig.json",
                    r#"{"compilerOptions":{"baseUrl":".","paths":{"@lib/*":["src/*"]}}}"#,
                ),
                ("src/helper.ts", ""),
            ],
        ),
        options,
    );
    assert_eq!(
        resolver.resolve(&base, "@lib/helper").unwrap().path(),
        base.join("src/helper.ts")
    );
}

#[test]
fn package_exports_are_resolved_using_explicit_conditions() {
    let base = root();
    let resolver = modules::resolver(
        Snapshot::new(
            base.clone(),
            &[
                (
                    "node_modules/demo/package.json",
                    r#"{"name":"demo","exports":{".":{"import":"./esm.js","default":"./fallback.js"}}}"#,
                ),
                ("node_modules/demo/esm.js", ""),
                ("node_modules/demo/fallback.js", ""),
            ],
        ),
        modules::options(),
    );
    assert_eq!(
        resolver.resolve(&base, "demo").unwrap().path(),
        base.join("node_modules/demo/esm.js")
    );
}

#[test]
fn configured_extension_priority_is_not_current_conservative_ambiguity_policy() {
    let base = root();
    let resolver = modules::resolver(
        Snapshot::new(base.clone(), &[("lib.ts", ""), ("lib.js", "")]),
        modules::options(),
    );
    assert_eq!(
        resolver.resolve(&base, "./lib").unwrap().path(),
        base.join("lib.ts")
    );
}

#[test]
fn missing_and_outside_snapshot_requests_never_resolve() {
    let base = root();
    let resolver = modules::resolver(
        Snapshot::new(base.clone(), &[("lib.ts", "")]),
        modules::options(),
    );
    assert!(resolver.resolve(&base, "./missing").is_err());
    assert!(resolver.resolve(&base, "../outside").is_err());
}
