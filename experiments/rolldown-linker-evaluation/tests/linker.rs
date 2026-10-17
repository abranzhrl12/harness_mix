use harness_rolldown_linker_evaluation::evaluate;
use serde_json::Value;

async fn run(files: &[(&str, &str)]) -> Value {
    let root = tempfile::tempdir().unwrap();
    let report = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        evaluate(root.path(), files),
    )
    .await
    .expect("probe deadline")
    .expect("probe setup");
    assert!(
        !root.path().join("app.ts").exists(),
        "sources must exist only in the snapshot"
    );
    assert!(
        !root.path().join("dist").exists(),
        "generate must not write a bundle"
    );
    assert_eq!(report["snapshot"]["deniedWrites"], 0, "{report}");
    report
}
fn code(report: &Value) -> String {
    report["chunks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["code"].as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn star_chain_links_with_no_node_or_source_files_on_disk() {
    let report = run(&[
        (
            "leaf.ts",
            "export function internal(){return 'LEAF_SENTINEL'}",
        ),
        ("middle.ts", "export {internal as run} from './leaf';"),
        ("barrel.ts", "export * from './middle';"),
        (
            "app.ts",
            "import {run} from './barrel'; export const result=run();",
        ),
    ])
    .await;
    assert_eq!(report["status"], "ok", "{report}");
    assert!(code(&report).contains("LEAF_SENTINEL"));
    let modules = report["modules"].as_array().unwrap();
    assert!(modules.iter().any(|m| m["path"] == "leaf.ts"));
}

#[tokio::test]
async fn star_conflicts_are_not_silently_resolved_to_one_definition() {
    let report = run(&[
        ("one.ts", "export function run(){return 'ONE'}"),
        ("two.ts", "export function run(){return 'TWO'}"),
        ("barrel.ts", "export * from './one'; export * from './two';"),
        (
            "app.ts",
            "import {run} from './barrel'; export const result=run();",
        ),
    ])
    .await;
    assert_eq!(report["status"], "link_error", "{report}");
}

#[tokio::test]
async fn diamonds_of_the_same_original_binding_are_not_ambiguous() {
    let report = run(&[
        ("leaf.ts", "export function run(){return 'SAME_BINDING'}"),
        ("one.ts", "export * from './leaf';"),
        ("two.ts", "export * from './leaf';"),
        ("barrel.ts", "export * from './one'; export * from './two';"),
        (
            "app.ts",
            "import {run} from './barrel'; export const result=run();",
        ),
    ])
    .await;
    assert_eq!(report["status"], "ok", "{report}");
    assert!(code(&report).contains("SAME_BINDING"));
}

#[tokio::test]
async fn cycles_can_have_an_independent_valid_star_branch() {
    let report = run(&[
        ("leaf.ts", "export function run(){return 'CYCLE_LEAF'}"),
        ("one.ts", "export * from './barrel';"),
        (
            "barrel.ts",
            "export * from './one'; export * from './leaf';",
        ),
        (
            "app.ts",
            "import {run} from './barrel'; export const result=run();",
        ),
    ])
    .await;
    assert_eq!(report["status"], "ok", "{report}");
    assert!(code(&report).contains("CYCLE_LEAF"));
}

#[tokio::test]
async fn default_and_namespace_bindings_are_handled_by_the_linker() {
    let report = run(&[
        ("leaf.ts", "export default function d(){return 'DEFAULT_LEAF'} export function named(){return 'NAMED_LEAF'}"),
        ("app.ts", "import d,* as ns from './leaf'; export const result=[d(),ns.named()];"),
    ]).await;
    assert_eq!(report["status"], "ok", "{report}");
    assert!(code(&report).contains("DEFAULT_LEAF"));
    assert!(code(&report).contains("NAMED_LEAF"));
}

#[tokio::test]
async fn package_exports_use_the_snapshot_manifest() {
    let report = run(&[
        (
            "node_modules/pkg/package.json",
            r#"{"name":"pkg","exports":{".":{"import":"./esm.js","require":"./cjs.js"}}}"#,
        ),
        (
            "node_modules/pkg/esm.js",
            "export function run(){return 'ESM_LEAF'}",
        ),
        ("node_modules/pkg/cjs.js", "exports.run=()=> 'CJS_LEAF';"),
        (
            "app.ts",
            "import {run} from 'pkg'; export const result=run();",
        ),
    ])
    .await;
    assert_eq!(report["status"], "ok", "{report}");
    assert!(code(&report).contains("ESM_LEAF"));
    assert!(!code(&report).contains("CJS_LEAF"));
}

#[tokio::test]
async fn inherited_tsconfig_paths_can_be_resolved_inside_the_snapshot() {
    let report = run(&[
        (
            "configs/base.json",
            r#"{"compilerOptions":{"baseUrl":"..","paths":{"@lib":["leaf"]}}}"#,
        ),
        ("tsconfig.json", r#"{"extends":"./configs/base.json"}"#),
        ("leaf.ts", "export function run(){return 'ALIAS_LEAF'}"),
        (
            "app.ts",
            "import {run} from '@lib'; export const result=run();",
        ),
    ])
    .await;
    assert_eq!(report["status"], "ok", "{report}");
    assert!(code(&report).contains("ALIAS_LEAF"));
}

#[tokio::test]
async fn missing_files_and_imports_outside_the_snapshot_are_not_read_from_disk() {
    let report = run(&[(
        "app.ts",
        "import {run} from '../outside'; export const result=run();",
    )])
    .await;
    assert_eq!(report["status"], "link_error", "{report}");
}

#[tokio::test]
async fn a_real_host_file_is_not_a_fallback_for_a_missing_snapshot_module() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("hidden.ts"),
        "export function run(){return 'HOST_MUST_NOT_LOAD'}",
    )
    .unwrap();
    let report = evaluate(
        root.path(),
        &[(
            "app.ts",
            "import {run} from './hidden'; export const result=run();",
        )],
    )
    .await
    .unwrap();
    assert_eq!(report["status"], "link_error", "{report}");
}

#[tokio::test]
async fn input_path_and_source_budgets_are_rejected_before_building() {
    let root = tempfile::tempdir().unwrap();
    assert!(
        evaluate(root.path(), &[("../outside.ts", "")])
            .await
            .is_err()
    );
    assert!(
        evaluate(root.path(), &[("app.ts", &" ".repeat(256 * 1024 + 1))])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn public_metadata_keeps_star_markers_and_rendered_chunks_omit_the_barrel() {
    let report = run(&[
        ("leaf.ts", "export function run(){return 'LEAF_SENTINEL'}"),
        ("barrel.ts", "export * from './leaf';"),
        (
            "app.ts",
            "import {run} from './barrel'; export const result=run();",
        ),
    ])
    .await;
    assert_eq!(report["status"], "ok", "{report}");
    let barrel = report["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["path"] == "barrel.ts")
        .unwrap();
    assert_eq!(barrel["exports"], serde_json::json!(["*"]));
    let rendered = report["chunks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["sourceModules"].as_array().unwrap())
        .collect::<Vec<_>>();
    assert!(rendered.iter().any(|p| **p == "leaf.ts"));
    assert!(!rendered.iter().any(|p| **p == "barrel.ts"));
}
