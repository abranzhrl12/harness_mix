#![cfg(windows)]
use agent_harness::adapters::{self, index_store::sync_file};
use harness_domain::{
    contracts::{ExploreArgs, Request},
    execution::OperationContext,
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn fixture(root: &Path) {
    fs::create_dir(root.join(".codegraph")).unwrap();
    let db = db(root);
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute_batch(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0); PRAGMA journal_mode=WAL;",
    )
    .unwrap();
}
fn db(root: &Path) -> Connection {
    Connection::open(root.join(".codegraph/codegraph.db")).unwrap()
}
fn publish(root: &Path, path: &str, source: &str) {
    fs::write(root.join(path), source).unwrap();
    sync_file(root, path, &OperationContext::default()).unwrap_or_else(|e| panic!("{path}: {e:?}"));
}
fn calls(root: &Path) -> Vec<(String, String)> {
    let db = db(root);
    let mut query = db.prepare("SELECT t.file_path,t.name FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE s.name='caller' AND e.kind='calls' ORDER BY t.file_path,t.name").unwrap();
    query
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
fn metadata(root: &Path) -> Value {
    let text: String = db(root).query_row("SELECT e.metadata FROM edges e JOIN nodes s ON s.id=e.source WHERE s.name='caller' AND e.kind='calls'", [], |r| r.get(0)).unwrap();
    serde_json::from_str(&text).unwrap()
}
fn strict(root: &Path) -> Value {
    let mut args: ExploreArgs = serde_json::from_value(json!({"query":"caller"})).unwrap();
    args.require_matching_files = Some(true);
    adapters::application(root)
        .unwrap()
        .execute(Request::Explore(args))
}
const APP: &str =
    "import {api as execute} from './barrel.mjs'; export function caller(){return execute();}";
type Case = (
    Vec<(&'static str, &'static str)>,
    Option<(&'static str, &'static str)>,
);
fn cases() -> Vec<Case> {
    vec![
        (
            vec![
                ("leaf.mjs", "export function api(){return 'leaf.mjs:api'}"),
                ("barrel.mjs", "export * from './leaf.mjs';"),
            ],
            Some(("leaf.mjs", "api")),
        ),
        (
            vec![
                (
                    "leaf.mjs",
                    "function original(){return 'leaf.mjs:original'} export {original as first,original as second};",
                ),
                ("left.mjs", "export {first as api} from './leaf.mjs';"),
                ("right.mjs", "export {second as api} from './leaf.mjs';"),
                (
                    "barrel.mjs",
                    "export * from './left.mjs'; export * from './right.mjs';",
                ),
            ],
            Some(("leaf.mjs", "original")),
        ),
        (
            vec![
                ("one.mjs", "export function api(){return 'one.mjs:api'}"),
                ("two.mjs", "export function api(){return 'two.mjs:api'}"),
                (
                    "barrel.mjs",
                    "export * from './one.mjs'; export * from './two.mjs';",
                ),
            ],
            None,
        ),
        (
            vec![
                ("one.mjs", "export function api(){return 'one.mjs:api'}"),
                ("two.mjs", "export function api(){return 'two.mjs:api'}"),
                (
                    "barrel.mjs",
                    "export * from './two.mjs'; export * from './one.mjs';",
                ),
            ],
            None,
        ),
        (
            vec![
                (
                    "leaf.mjs",
                    "export function one(){return 'leaf.mjs:one'} export function two(){return 'leaf.mjs:two'}",
                ),
                ("left.mjs", "export {one as api} from './leaf.mjs';"),
                ("right.mjs", "export {two as api} from './leaf.mjs';"),
                (
                    "barrel.mjs",
                    "export * from './left.mjs'; export * from './right.mjs';",
                ),
            ],
            None,
        ),
        (
            vec![
                ("one.mjs", "export function api(){return 'one.mjs:api'}"),
                ("two.mjs", "export function api(){return 'two.mjs:api'}"),
                (
                    "barrel.mjs",
                    "export * from './one.mjs'; export * from './two.mjs'; export {api} from './one.mjs';",
                ),
            ],
            Some(("one.mjs", "api")),
        ),
        (
            vec![
                ("leaf.mjs", "export function api(){return 'leaf.mjs:api'}"),
                (
                    "barrel.mjs",
                    "export * from './leaf.mjs'; function own(){return 'barrel.mjs:own'} export {own as api};",
                ),
            ],
            Some(("barrel.mjs", "own")),
        ),
        (
            vec![
                ("leaf.mjs", "export function api(){return 'leaf.mjs:api'}"),
                ("cycle.mjs", "export * from './barrel.mjs';"),
                (
                    "barrel.mjs",
                    "export * from './cycle.mjs'; export * from './leaf.mjs';",
                ),
            ],
            Some(("leaf.mjs", "api")),
        ),
        (
            vec![
                ("leaf.mjs", "export function api(){return 'leaf.mjs:api'}"),
                ("cycle.mjs", "export * from './barrel.mjs';"),
                (
                    "barrel.mjs",
                    "export * from './leaf.mjs'; export * from './cycle.mjs';",
                ),
            ],
            Some(("leaf.mjs", "api")),
        ),
        (
            vec![
                ("cycle.mjs", "export * from './barrel.mjs';"),
                ("barrel.mjs", "export * from './cycle.mjs';"),
            ],
            None,
        ),
        (
            vec![
                (
                    "leaf.mjs",
                    "export default function api(){return 'default must not be inherited'}",
                ),
                ("barrel.mjs", "export * from './leaf.mjs';"),
            ],
            None,
        ),
    ]
}

#[test]
fn star_cli_publishes_original_binding_with_an_empty_path() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    publish(root.path(), "leaf.mjs", "export function api(){}");
    publish(root.path(), "barrel.mjs", "export * from './leaf.mjs';");
    fs::write(root.path().join("app.mjs"), APP).unwrap();
    let binary = std::env::var_os("HARNESS_NATIVE_BINARY")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_agent-harness").into());
    let output = Command::new(binary)
        .env("PATH", "")
        .args(["--root"])
        .arg(root.path())
        .args(["--sync-file", "app.mjs"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["committed"], true);
    assert_eq!(calls(root.path()), [("leaf.mjs".into(), "api".into())]);
    assert_eq!(
        metadata(root.path())["exportEngine"],
        "oxc-module-record-0.90.0-resolve-export-v2"
    );
}

#[test]
fn star_matrix_compares_original_bindings_and_never_picks_a_conflicting_branch() {
    for (files, expected) in cases() {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path());
        for (path, source) in &files {
            publish(root.path(), path, source);
        }
        publish(root.path(), "app.mjs", APP);
        let expected = expected
            .into_iter()
            .map(|(p, n)| (p.to_string(), n.to_string()))
            .collect::<Vec<_>>();
        assert_eq!(calls(root.path()), expected, "{files:?}");
        if !expected.is_empty() {
            let result = strict(root.path());
            assert_eq!(result["state"], "ok", "{result}");
            assert_eq!(
                metadata(root.path())["exportEngine"],
                "oxc-module-record-0.90.0-resolve-export-v2"
            );
        }
    }
}

#[test]
fn no_export_branch_hash_is_recorded_and_an_added_export_removes_the_old_edge() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    publish(root.path(), "leaf.mjs", "export function api(){}");
    publish(root.path(), "other.mjs", "export function unrelated(){}");
    publish(
        root.path(),
        "barrel.mjs",
        "export * from './leaf.mjs'; export * from './other.mjs';",
    );
    publish(root.path(), "app.mjs", APP);
    assert_eq!(calls(root.path()).len(), 1);
    assert!(
        metadata(root.path())["exportContributors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["path"] == "other.mjs")
    );
    fs::write(root.path().join("other.mjs"), "export function api(){}").unwrap();
    assert_eq!(strict(root.path())["diagnostics"][0]["code"], "INDEX_STALE");
    sync_file(root.path(), "other.mjs", &OperationContext::default()).unwrap();
    assert!(calls(root.path()).is_empty());
    publish(root.path(), "other.mjs", "export function unrelated(){}\n");
    assert_eq!(calls(root.path()).len(), 1);
    fs::remove_file(root.path().join("other.mjs")).unwrap();
    sync_file(root.path(), "other.mjs", &OperationContext::default()).unwrap();
    assert!(
        calls(root.path()).is_empty(),
        "missing branch must not be hidden by a successful one"
    );
    publish(root.path(), "other.mjs", "export {};");
    assert_eq!(calls(root.path()).len(), 1);
}

#[test]
fn stale_or_unsupported_branches_cannot_be_hidden_by_a_successful_branch() {
    for other in [
        "export const api=()=>{};",
        "export * as api from './leaf.mjs';",
    ] {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path());
        publish(root.path(), "leaf.mjs", "export function api(){}");
        publish(root.path(), "other.mjs", other);
        publish(
            root.path(),
            "barrel.mjs",
            "export * from './leaf.mjs'; export * from './other.mjs';",
        );
        publish(root.path(), "app.mjs", APP);
        assert!(calls(root.path()).is_empty(), "{other}");
    }
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    publish(root.path(), "leaf.mjs", "export function api(){}");
    publish(root.path(), "other.mjs", "export function unrelated(){}");
    fs::write(root.path().join("other.mjs"), "export function api(){}").unwrap();
    publish(
        root.path(),
        "barrel.mjs",
        "export * from './leaf.mjs'; export * from './other.mjs';",
    );
    publish(root.path(), "app.mjs", APP);
    assert!(calls(root.path()).is_empty());
}

#[test]
fn type_only_star_is_deferred_and_named_types_do_not_shadow_runtime_stars() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    publish(root.path(), "leaf.ts", "export function api(){}");
    publish(
        root.path(),
        "types.ts",
        "export interface api {} export interface Api {}",
    );
    fs::write(
        root.path().join("barrel.ts"),
        "export type * from './types';",
    )
    .unwrap();
    assert_eq!(
        sync_file(root.path(), "barrel.ts", &OperationContext::default())
            .unwrap_err()
            .0,
        "EXTRACTION_DEFERRED"
    );
    assert_eq!(
        db(root.path())
            .query_row(
                "SELECT count(*) FROM files WHERE path='barrel.ts'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let barrel = "export type {Api as api} from './types'; export * from './leaf';";
    publish(root.path(), "barrel.ts", barrel);
    publish(
        root.path(),
        "app.ts",
        "import {api} from './barrel'; export function caller(){api();}",
    );
    assert_eq!(calls(root.path()).len(), 1, "{barrel}");
}

#[test]
fn branch_work_and_provenance_budgets_roll_back_publication() {
    for repeats in [256, 257] {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path());
        publish(root.path(), "leaf.mjs", "export function api(){}");
        publish(
            root.path(),
            "barrel.mjs",
            &"export * from './leaf.mjs';".repeat(repeats),
        );
        fs::write(root.path().join("app.mjs"), APP).unwrap();
        assert_eq!(
            sync_file(root.path(), "app.mjs", &OperationContext::default())
                .unwrap_err()
                .0,
            "SYNC_LIMIT"
        );
        assert_eq!(
            db(root.path())
                .query_row("SELECT count(*) FROM files WHERE path='app.mjs'", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    let mut barrel = String::new();
    for i in 0..16 {
        let path = format!("branch{i}.mjs");
        publish(
            root.path(),
            &path,
            if i == 0 {
                "export function api(){}"
            } else {
                "export {};"
            },
        );
        barrel.push_str(&format!("export * from './{path}';"));
    }
    publish(root.path(), "barrel.mjs", &barrel);
    fs::write(root.path().join("app.mjs"), APP).unwrap();
    assert_eq!(
        sync_file(root.path(), "app.mjs", &OperationContext::default())
            .unwrap_err()
            .0,
        "SYNC_LIMIT"
    );
}

#[test]
#[ignore = "requires HARNESS_CODEGRAPH_NODE and built HARNESS_ROLLDOWN_PROBE; synthetic JS only"]
fn star_binding_matrix_matches_native_esm_and_rolldown_generated_bundles() {
    let node = std::env::var("HARNESS_CODEGRAPH_NODE").expect("Node oracle required");
    let probe = std::env::var("HARNESS_ROLLDOWN_PROBE").expect("Rolldown probe binary required");
    let script = r#"
(async()=>{try {
 const url = process.argv[1]==='file' ? require('url').pathToFileURL(process.argv[2]).href : 'data:text/javascript;base64,'+Buffer.from(process.argv[2]).toString('base64');
 const module = await import(url); process.stdout.write(JSON.stringify({value:module.caller()}));
} catch(e) {if(e.name!=='SyntaxError')throw e; process.stdout.write(JSON.stringify({linkError:true}));}})().catch(e=>{console.error(e.message);process.exitCode=1});
"#;
    for (mut files, expected) in cases() {
        files.push(("app.mjs", APP));
        let root = tempfile::tempdir().unwrap();
        fixture(root.path());
        for (path, source) in &files {
            publish(root.path(), path, source);
        }
        let native = calls(root.path()).first().map(|(p, n)| format!("{p}:{n}"));
        assert_eq!(native, expected.map(|(p, n)| format!("{p}:{n}")));
        let oracle = |kind: &str, input: &str| {
            let output = Command::new(&node)
                .env("NODE_OPTIONS", "")
                .env("NODE_PATH", "")
                .args(["-e", script, kind, input])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            serde_json::from_slice::<Value>(&output.stdout).unwrap()
        };
        let esm = oracle("file", root.path().join("app.mjs").to_str().unwrap());
        assert_eq!(esm["value"].as_str(), native.as_deref(), "{files:?}: {esm}");
        let pairs = files
            .iter()
            .map(|(p, s)| (if *p == "app.mjs" { "app.ts" } else { p }, s))
            .collect::<Vec<_>>();
        let output = Command::new(&probe)
            .env("PATH", "")
            .args(["--fixture-json", &serde_json::to_string(&pairs).unwrap()])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        if native.is_some() {
            assert_eq!(report["status"], "ok", "{report}");
            assert_eq!(report["chunks"].as_array().unwrap().len(), 1);
            let bundled = oracle("bundle", report["chunks"][0]["code"].as_str().unwrap());
            assert_eq!(
                bundled["value"].as_str(),
                native.as_deref(),
                "{files:?}: {bundled}"
            );
        } else {
            assert_eq!(esm["linkError"], true);
            assert_eq!(report["status"], "link_error", "{report}");
        }
    }
}
