#![cfg(windows)]
use agent_harness::adapters::{
    self,
    index_store::{sync_file, sync_file_with_tsconfig},
};
use harness_domain::{
    contracts::{ExploreArgs, Request},
    execution::OperationContext,
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{fs, path::Path};

fn fixture(root: &Path) {
    fs::create_dir_all(root.join(".codegraph")).unwrap();
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute_batch(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0); PRAGMA journal_mode=WAL;",
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    for file in ["one", "two"] {
        fs::write(
            root.join(format!("src/{file}.ts")),
            "export function run() {}\n",
        )
        .unwrap();
        sync_file(
            root,
            &format!("src/{file}.ts"),
            &OperationContext::default(),
        )
        .unwrap();
    }
}
fn selected(root: &Path) -> Result<Value, harness_domain::Failure> {
    sync_file_with_tsconfig(
        root,
        "app.ts",
        "tsconfig.json",
        &OperationContext::default(),
    )
}
fn config(root: &Path, target: &str) {
    fs::write(
        root.join("tsconfig.json"),
        json!({"compilerOptions":{"baseUrl":".","paths":{"@lib":[target]}}}).to_string(),
    )
    .unwrap();
}
fn targets(root: &Path) -> Vec<String> {
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    let mut query = db.prepare("SELECT t.file_path FROM edges e JOIN nodes t ON t.id=e.target JOIN nodes s ON s.id=e.source WHERE e.kind='calls' AND s.name='caller' ORDER BY t.file_path").unwrap();
    query
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn exact_wildcard_multiple_targets_and_baseurl_use_oxc_snapshot() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("tsconfig.json"), json!({"compilerOptions":{"baseUrl":"src","paths":{"@exact":["one"],"@lib/*":["missing/*","*"]}}}).to_string()).unwrap();
    fs::write(root.path().join("app.ts"), "import {run as a} from '@exact';\nimport {run as b} from '@lib/two';\nimport {run as c} from 'one';\nexport function caller(){\n a();\n b();\n c();\n}\n").unwrap();
    let result = selected(root.path()).unwrap();
    assert_eq!(result["committed"], true);
    assert_eq!(
        targets(root.path()),
        ["src/one.ts", "src/two.ts"],
        "{result}"
    );
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    let stamps: i64 = db.query_row("SELECT count(*) FROM edges WHERE json_extract(metadata,'$.tsconfig.path')='tsconfig.json' AND length(json_extract(metadata,'$.tsconfig.hash'))=64", [], |r| r.get(0)).unwrap();
    assert_eq!(stamps, 2);
}

#[test]
fn config_only_changes_and_deletion_rebind_unchanged_importers() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("app.ts"),
        "import {run} from '@lib'; export function caller(){ run(); }\n",
    )
    .unwrap();
    config(root.path(), "src/one");
    selected(root.path()).unwrap();
    assert_eq!(targets(root.path()), ["src/one.ts"]);
    config(root.path(), "src/two");
    let app = adapters::application(root.path()).unwrap();
    let mut args: ExploreArgs = serde_json::from_value(json!({"query":"caller"})).unwrap();
    args.require_matching_files = Some(true);
    let stale = app.execute(Request::Explore(args));
    assert_eq!(stale["diagnostics"][0]["code"], "INDEX_STALE", "{stale}");
    let result = sync_file(root.path(), "app.ts", &OperationContext::default()).unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(targets(root.path()), ["src/two.ts"]);
    fs::remove_file(root.path().join("tsconfig.json")).unwrap();
    agent_harness::adapters::index_store::sync_workspace(root.path(), &OperationContext::default())
        .unwrap();
    assert!(targets(root.path()).is_empty());
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM project_metadata WHERE key='native_tsconfig_v1'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn jsonc_and_local_extends_track_parent_freshness_and_overrides() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir(root.path().join("configs")).unwrap();
    fs::write(
        root.path().join("app.ts"),
        "import {run} from '@lib'; export function caller(){run();}\n",
    )
    .unwrap();
    fs::write(
        root.path().join("configs/base.json"),
        r#"{/* café */ "compilerOptions":{"baseUrl":"../src", "paths":{"@lib":["one",],},},}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("tsconfig.json"),
        "{\n// inherit\n\"extends\":\"./configs/base\",\n}",
    )
    .unwrap();
    selected(root.path()).unwrap();
    assert_eq!(targets(root.path()), ["src/one.ts"]);
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    let stamp: String = db
        .query_row(
            "SELECT value FROM project_metadata WHERE key='native_tsconfig_v1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&stamp).unwrap()["dependencies"][0]["path"],
        "configs/base.json"
    );
    drop(db);
    fs::write(
        root.path().join("configs/base.json"),
        r#"{"compilerOptions":{"baseUrl":"../src","paths":{"@lib":["two"]}}}"#,
    )
    .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let mut args: ExploreArgs = serde_json::from_value(json!({"query":"caller"})).unwrap();
    args.require_matching_files = Some(true);
    assert_eq!(
        app.execute(Request::Explore(args))["diagnostics"][0]["code"],
        "INDEX_STALE"
    );
    sync_file(root.path(), "app.ts", &OperationContext::default()).unwrap();
    assert_eq!(targets(root.path()), ["src/two.ts"]);
    fs::write(root.path().join("tsconfig.json"), r#"{"extends":"./configs/base.json","compilerOptions":{"baseUrl":".","paths":{"@lib":["src/one"]}}}"#).unwrap();
    selected(root.path()).unwrap();
    assert_eq!(targets(root.path()), ["src/one.ts"]);
    // Missing ancestors and cycles must roll back rather than silently drop aliases.
    for parent in [
        r#"{"extends":"../tsconfig.json"}"#,
        r#"{"extends":"./missing.json"}"#,
    ] {
        fs::write(root.path().join("configs/base.json"), parent).unwrap();
        assert!(selected(root.path()).is_err());
        assert_eq!(targets(root.path()), ["src/one.ts"]);
    }
}

#[test]
fn config_directory_relative_paths_and_relative_import_priority() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir(root.path().join("configs")).unwrap();
    fs::write(
        root.path().join("configs/tsconfig.json"),
        json!({"compilerOptions":{"baseUrl":"../src","paths":{"./one":["two"],"@lib":["one"]}}})
            .to_string(),
    )
    .unwrap();
    fs::write(
        root.path().join("app.ts"),
        "import {run} from './src/one'; export function caller(){run();}\n",
    )
    .unwrap();
    // Relative alias keys are explicitly unsupported, never allowed to override imports.
    assert!(
        sync_file_with_tsconfig(
            root.path(),
            "app.ts",
            "configs/tsconfig.json",
            &OperationContext::default()
        )
        .is_err()
    );
    fs::write(
        root.path().join("configs/tsconfig.json"),
        json!({"compilerOptions":{"baseUrl":"../src","paths":{"@lib":["two"]}}}).to_string(),
    )
    .unwrap();
    sync_file_with_tsconfig(
        root.path(),
        "app.ts",
        "configs/tsconfig.json",
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(targets(root.path()), ["src/one.ts"]);
}

#[test]
fn invalid_unsupported_escaping_and_sensitive_configs_never_publish() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("app.ts"),
        "import {run} from '@lib'; export function caller(){run();}\n",
    )
    .unwrap();
    config(root.path(), "src/one");
    selected(root.path()).unwrap();
    for value in [
        json!({"extends":"outside-private"}),
        json!({"references":[]}),
        json!({"compilerOptions":{"baseUrl":"../outside"}}),
        json!({"compilerOptions":{"paths":{"@lib":[".env/private"]}}}),
        json!({"compilerOptions":{"paths":{"@lib":["C:/outside"]}}}),
        json!({"compilerOptions":{"paths":{"@lib":"wrong-type"}}}),
    ] {
        fs::write(root.path().join("tsconfig.json"), value.to_string()).unwrap();
        let error = selected(root.path()).unwrap_err();
        assert!(!error.1.contains("outside-private"));
        assert_eq!(targets(root.path()), ["src/one.ts"]);
    }
    fs::write(root.path().join("tsconfig.json"), "{invalid-private}").unwrap();
    assert!(selected(root.path()).is_err());
    fs::write(
        root.path().join("tsconfig.json"),
        " ".repeat(256 * 1024 + 1),
    )
    .unwrap();
    assert!(selected(root.path()).is_err());
    assert_eq!(targets(root.path()), ["src/one.ts"]);
}

#[test]
fn hardlinked_config_and_missing_selection_are_refused() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("app.ts"), "export function caller(){}\n").unwrap();
    assert!(selected(root.path()).is_err());
    fs::write(root.path().join("original.json"), "{}").unwrap();
    fs::hard_link(
        root.path().join("original.json"),
        root.path().join("tsconfig.json"),
    )
    .unwrap();
    assert!(selected(root.path()).is_err());
}

#[test]
fn inherited_config_limits_and_hardlinks_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("app.ts"), "export function caller(){}\n").unwrap();
    fs::write(
        root.path().join("tsconfig.json"),
        r#"{"extends":"./base0"}"#,
    )
    .unwrap();
    for i in 0..8 {
        fs::write(
            root.path().join(format!("base{i}.json")),
            json!({"extends":format!("./base{}", i+1)}).to_string(),
        )
        .unwrap();
    }
    fs::write(root.path().join("base8.json"), "{}").unwrap();
    assert!(selected(root.path()).is_err(), "depth must be bounded");
    // Five individually valid files exceed the aggregate 1 MiB budget.
    for i in 0..5 {
        let mut text = if i == 4 {
            "{}".into()
        } else {
            json!({"extends":format!("./base{}", i+1)}).to_string()
        };
        text.push_str(&" ".repeat(220 * 1024));
        fs::write(root.path().join(format!("base{i}.json")), text).unwrap();
    }
    assert!(
        selected(root.path()).is_err(),
        "aggregate bytes must be bounded"
    );
    fs::remove_file(root.path().join("base0.json")).unwrap();
    fs::write(root.path().join("original.json"), "{}").unwrap();
    fs::hard_link(
        root.path().join("original.json"),
        root.path().join("base0.json"),
    )
    .unwrap();
    assert!(
        selected(root.path()).is_err(),
        "ancestor hardlinks must be refused"
    );
}

#[test]
fn cli_exposes_explicit_selection_without_node() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("app.ts"),
        "import {run} from '@lib'; export function caller(){run();}\n",
    )
    .unwrap();
    config(root.path(), "src/one");
    let binary = std::env::var_os("HARNESS_NATIVE_BINARY")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_agent-harness").into());
    let output = std::process::Command::new(binary)
        .env("PATH", "")
        .arg("--root")
        .arg(root.path())
        .args(["--sync-file", "app.ts", "--tsconfig", "tsconfig.json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(targets(root.path()), ["src/one.ts"]);
}

#[test]
fn node_path_cannot_make_a_missing_alias_bind_an_unrelated_module() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("src/@lib.ts"), "export function run(){}\n").unwrap();
    sync_file(root.path(), "src/@lib.ts", &OperationContext::default()).unwrap();
    fs::write(
        root.path().join("app.ts"),
        "import {run} from '@lib'; export function caller(){run();}\n",
    )
    .unwrap();
    fs::write(
        root.path().join("tsconfig.json"),
        json!({"compilerOptions":{"baseUrl":"src","paths":{"@lib":["missing"]}}}).to_string(),
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_agent-harness"))
        .env("NODE_PATH", r"C:\__harness_codegraph_snapshot\src")
        .env("PATH", "")
        .arg("--root")
        .arg(root.path())
        .args(["--sync-file", "app.ts", "--tsconfig", "tsconfig.json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(targets(root.path()).is_empty());
}

#[test]
#[ignore = "requires HARNESS_CODEGRAPH_PACKAGE v1.6.2 and HARNESS_CODEGRAPH_NODE; provider oracle"]
fn alias_targets_match_installed_provider_for_supported_cases() {
    let package = std::env::var("HARNESS_CODEGRAPH_PACKAGE").expect("provider package required");
    let node = std::env::var("HARNESS_CODEGRAPH_NODE").expect("provider Node required");
    let script = r#"
const path = require('path');
const api = require(path.join(process.argv[1], 'dist/resolution/import-resolver.js'));
const aliases = require(path.join(process.argv[1], 'dist/resolution/path-aliases.js')).loadProjectAliases(process.argv[2]);
const input = JSON.parse(process.argv[3]);
const ctx = {getProjectRoot:()=>process.argv[2],getProjectAliases:()=>aliases,fileExists:p=>input.files.includes(p)};
process.stdout.write(JSON.stringify(api.resolveImportPath(input.module,'app.ts','typescript',ctx)));
"#;
    for (module, options) in [
        (
            "@exact",
            json!({"compilerOptions":{"baseUrl":"src","paths":{"@exact":["one"]}}}),
        ),
        (
            "@lib/two",
            json!({"compilerOptions":{"baseUrl":"src","paths":{"@lib/*":["missing/*","*"]}}}),
        ),
        (
            "@missing",
            json!({"compilerOptions":{"baseUrl":"src","paths":{"@missing":["absent"]}}}),
        ),
        ("@exact", json!({"extends":"./configs/base"})),
        (
            "@exact",
            json!({"extends":"./configs/base.json","compilerOptions":{"baseUrl":"."}}),
        ),
        (
            "@exact",
            json!({"extends":"./configs/base.json","compilerOptions":{"paths":{"@exact":["two"]}}}),
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path());
        fs::create_dir(root.path().join("configs")).unwrap();
        fs::write(root.path().join("configs/base.json"), r#"{/* inherited JSONC */"compilerOptions":{"baseUrl":"../src","paths":{"@exact":["one",],},},}"#).unwrap();
        fs::write(
            root.path().join("tsconfig.json"),
            format!("// JSONC root\n{options}"),
        )
        .unwrap();
        fs::write(
            root.path().join("app.ts"),
            format!("import {{run}} from '{module}'; export function caller(){{run();}}\n"),
        )
        .unwrap();
        selected(root.path()).unwrap();
        let output = std::process::Command::new(&node)
            .env("NODE_OPTIONS", "")
            .env("NODE_PATH", "")
            .arg("-e")
            .arg(script)
            .arg(&package)
            .arg(root.path())
            .arg(json!({"module":module,"files":["src/one.ts","src/two.ts"]}).to_string())
            .output()
            .unwrap();
        assert!(output.status.success(), "provider oracle failed");
        let expected: Option<String> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(targets(root.path()).first(), expected.as_ref(), "{module}");
    }
}
