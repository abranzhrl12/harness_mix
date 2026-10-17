#![cfg(windows)]
use agent_harness::adapters::{self, index_store::sync_file};
use harness_domain::{
    contracts::{ExploreArgs, Request},
    execution::OperationContext,
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{fs, path::Path};

fn fixture(root: &Path) {
    fs::create_dir(root.join(".codegraph")).unwrap();
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
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
    sync_file(root, path, &OperationContext::default()).unwrap();
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
fn explore(root: &Path) -> Value {
    let mut args: ExploreArgs = serde_json::from_value(json!({"query":"caller"})).unwrap();
    args.require_matching_files = Some(true);
    adapters::application(root)
        .unwrap()
        .execute(Request::Explore(args))
}
const APP: &str = "import {api as execute} from './barrel'; export function caller(){execute();}\n";

#[test]
fn oxc_local_export_alias_and_named_chains_reach_original_function() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    publish(
        root.path(),
        "lib.ts",
        "function internal(){} export {internal as original};",
    );
    publish(
        root.path(),
        "middle.ts",
        "import {original as local} from './lib'; export {local as renamed};",
    );
    publish(
        root.path(),
        "barrel.ts",
        "export {renamed as api} from './middle';",
    );
    publish(root.path(), "app.ts", APP);
    assert_eq!(calls(root.path()), [("lib.ts".into(), "internal".into())]);
    let metadata: String = db(root.path()).query_row("SELECT e.metadata FROM edges e JOIN nodes s ON s.id=e.source WHERE s.name='caller' AND e.kind='calls'", [], |r| r.get(0)).unwrap();
    let metadata: Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(metadata["refName"], "execute");
    assert_eq!(
        metadata["exportEngine"],
        "oxc-module-record-0.90.0-named-chain-v1"
    );
    let contributors = metadata["exportContributors"].as_array().unwrap();
    assert_eq!(
        contributors
            .iter()
            .map(|c| c["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["barrel.ts", "middle.ts", "lib.ts"]
    );
    assert!(
        contributors
            .iter()
            .all(|c| c["hash"].as_str().unwrap().len() == 64)
    );
    let result = explore(root.path());
    assert!(
        result["diagnostics"].as_array().unwrap().is_empty(),
        "{result}"
    );
}

#[test]
fn barrel_only_change_is_stale_then_rebinds_and_deletion_recovers_pending_refs() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    publish(root.path(), "one.ts", "export function run(){}");
    publish(root.path(), "two.ts", "export function run(){}");
    publish(
        root.path(),
        "barrel.ts",
        "export {run as api} from './one';",
    );
    publish(root.path(), "app.ts", APP);
    assert_eq!(calls(root.path()), [("one.ts".into(), "run".into())]);
    fs::write(
        root.path().join("barrel.ts"),
        "export {run as api} from './two';",
    )
    .unwrap();
    assert_eq!(
        explore(root.path())["diagnostics"][0]["code"],
        "INDEX_STALE"
    );
    sync_file(root.path(), "barrel.ts", &OperationContext::default()).unwrap();
    assert_eq!(calls(root.path()), [("two.ts".into(), "run".into())]);
    fs::remove_file(root.path().join("two.ts")).unwrap();
    sync_file(root.path(), "two.ts", &OperationContext::default()).unwrap();
    assert!(calls(root.path()).is_empty());
    publish(root.path(), "two.ts", "export function run(){}\n");
    assert_eq!(calls(root.path()), [("two.ts".into(), "run".into())]);
    fs::remove_file(root.path().join("barrel.ts")).unwrap();
    sync_file(root.path(), "barrel.ts", &OperationContext::default()).unwrap();
    assert!(calls(root.path()).is_empty());
}

#[test]
fn cycles_types_default_namespace_stars_and_nonfunctions_never_guess_targets() {
    for barrel in [
        "export {api} from './barrel';",
        "export type {run as api} from './lib';",
        "export {type run as api} from './lib';",
        "export {default as api} from './lib';",
        "export * as api from './lib';",
        "export * from './lib';",
        "export {missing as api} from './lib';",
        "const api=()=>{}; export {api};",
        "function api(){}", // not exported: no global homonym fallback
        "import type {run as api} from './lib'; export {api};",
    ] {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path());
        publish(
            root.path(),
            "lib.ts",
            "export function run(){} export function api(){} export default run;",
        );
        publish(root.path(), "barrel.ts", barrel);
        publish(root.path(), "app.ts", APP);
        assert!(calls(root.path()).is_empty(), "{barrel}");
    }
}

#[test]
fn stale_intermediate_does_not_publish_a_guessed_target() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    publish(root.path(), "lib.ts", "export function run(){}");
    publish(
        root.path(),
        "barrel.ts",
        "export {run as api} from './lib';",
    );
    fs::write(
        root.path().join("barrel.ts"),
        "export {missing as api} from './lib';",
    )
    .unwrap();
    publish(root.path(), "app.ts", APP);
    assert!(calls(root.path()).is_empty());
}

#[test]
fn excessive_chain_rolls_back_instead_of_publishing_partial_resolution() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    publish(root.path(), "leaf.ts", "export function api(){}");
    for i in (0..16).rev() {
        let next = if i == 15 {
            "leaf".into()
        } else {
            format!("hop{}", i + 1)
        };
        publish(
            root.path(),
            &format!("hop{i}.ts"),
            &format!("export {{api}} from './{next}';"),
        );
    }
    publish(root.path(), "barrel.ts", "export {api} from './hop0';");
    fs::write(root.path().join("app.ts"), APP).unwrap();
    let error = sync_file(root.path(), "app.ts", &OperationContext::default()).unwrap_err();
    assert_eq!(error.0, "SYNC_LIMIT");
    let nodes: i64 = db(root.path())
        .query_row(
            "SELECT count(*) FROM nodes WHERE file_path='app.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(nodes, 0);
}

#[test]
fn corrupted_provenance_is_rejected_before_reading_external_paths() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    publish(root.path(), "barrel.ts", "export function api(){}");
    publish(root.path(), "app.ts", APP);
    db(root.path()).execute("UPDATE edges SET metadata=json_set(metadata,'$.exportContributors',json(?1)) WHERE kind='calls'", [json!([{"path":"../outside-private.ts","hash":"a".repeat(64)}]).to_string()]).unwrap();
    let result = explore(root.path());
    assert_eq!(
        result["diagnostics"][0]["code"], "GRAPH_CONTRACT",
        "{result}"
    );
    assert!(!result.to_string().contains("outside-private"));
}

#[test]
#[ignore = "requires CodeGraph v1.6.2 package and Node; indexes isolated temporary fixtures"]
fn named_reexport_targets_match_full_provider_indexing() {
    let package = std::env::var("HARNESS_CODEGRAPH_PACKAGE").expect("provider package required");
    let node = std::env::var("HARNESS_CODEGRAPH_NODE").expect("provider Node required");
    let script = r#"
const {CodeGraph} = require(require('path').join(process.argv[1], 'dist/index.js'));
(async () => {
  const graph = CodeGraph.initSync(process.argv[2]);
  try { await graph.indexAll(); } finally { graph.close(); }
})().catch(e => { console.error(e.message); process.exitCode=1; });
"#;
    for (library, middle, barrel) in [
        (
            "export function run(){}",
            "",
            "export {run as api} from './lib';",
        ),
        (
            "export function run(){}",
            "export {run as original} from './lib';",
            "export {original as api} from './middle';",
        ),
        (
            "function run(){} export {run as original};",
            "export {original as renamed} from './lib';",
            "export {renamed as api} from './middle';",
        ),
    ] {
        let native = tempfile::tempdir().unwrap();
        let provider = tempfile::tempdir().unwrap();
        fixture(native.path());
        for (path, source) in [
            ("lib.ts", library),
            ("middle.ts", middle),
            ("barrel.ts", barrel),
            ("app.ts", APP),
        ] {
            publish(native.path(), path, source);
            fs::write(provider.path().join(path), source).unwrap();
        }
        let output = std::process::Command::new(&node)
            .env("NODE_OPTIONS", "")
            .env("NODE_PATH", "")
            .args(["--liftoff-only", "-e", script])
            .arg(&package)
            .arg(provider.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "provider fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            calls(native.path()),
            calls(provider.path()),
            "{library} / {middle} / {barrel}"
        );
        assert_eq!(calls(native.path()).len(), 1);
    }
}
