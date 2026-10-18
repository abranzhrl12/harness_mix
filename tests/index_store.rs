#![cfg(windows)]
use agent_harness::adapters::index_store::{sync_file, sync_workspace};
use harness_domain::execution::OperationContext;
use rusqlite::Connection;
use std::{fs, path::Path, process::Command};

fn fixture(root: &Path) {
    fs::create_dir_all(root.join(".codegraph")).unwrap();
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0)",
        [],
    )
    .unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL").unwrap();
}
fn db(root: &Path) -> Connection {
    Connection::open(root.join(".codegraph/codegraph.db")).unwrap()
}
fn sync(root: &Path, path: &str) -> serde_json::Value {
    sync_file(root, path, &OperationContext::default()).unwrap()
}
fn count(root: &Path, query: &str) -> i64 {
    db(root).query_row(query, [], |r| r.get(0)).unwrap()
}

#[test]
fn oxc_emitted_specifiers_rebind_between_ts_source_and_existing_js() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("lib.ts"), "export function run() {}").unwrap();
    sync(root.path(), "lib.ts");
    fs::write(
        root.path().join("app.ts"),
        "import {run as execute} from './lib.js'; export function caller() { execute(); }",
    )
    .unwrap();
    sync(root.path(), "app.ts");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib.ts' AND json_extract(e.metadata,'$.moduleEngine')='oxc-resolver-11.0.0-index-snapshot'"
        ),
        1
    );
    fs::write(root.path().join("lib.js"), "export function run() {}").unwrap();
    sync(root.path(), "lib.js");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib.js'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib.ts'"
        ),
        0
    );
    fs::remove_file(root.path().join("lib.js")).unwrap();
    sync(root.path(), "lib.js");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib.ts'"
        ),
        1
    );
}

#[test]
fn oxc_relative_imports_cross_js_ts_family_and_follow_directory_priority() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir(root.path().join("lib")).unwrap();
    fs::write(root.path().join("lib/index.ts"), "export function run() {}").unwrap();
    sync(root.path(), "lib/index.ts");
    fs::write(
        root.path().join("app.tsx"),
        "import {run as execute} from './lib'; export function caller() { execute(); }",
    )
    .unwrap();
    sync(root.path(), "app.tsx");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib/index.ts'"
        ),
        1
    );
    fs::write(root.path().join("lib.tsx"), "export function run() {}").unwrap();
    sync(root.path(), "lib.tsx");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib.tsx'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib/index.ts'"
        ),
        0
    );
}

#[test]
fn oxc_cannot_read_tsconfig_packages_or_unindexed_module_files() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("lib.ts"), "export function run() {}").unwrap();
    sync(root.path(), "lib.ts");
    fs::write(root.path().join("lib.js"), "export function run() {}").unwrap(); // deliberately unindexed
    fs::write(
        root.path().join("tsconfig.json"),
        r#"{"compilerOptions":{"paths":{"@lib":["lib.ts"]}}}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("package.json"),
        r##"{"imports":{"#lib":"./lib.ts"}}"##,
    )
    .unwrap();
    for source in [
        "import {run} from '@lib'; export function caller() { run(); }",
        "import {run} from '#lib'; export function caller() { run(); }",
        "import {run} from 'external'; export function caller() { run(); }",
        "import {run} from './.env'; export function caller() { run(); }",
    ] {
        fs::write(root.path().join("app.ts"), source).unwrap();
        sync(root.path(), "app.ts");
        assert_eq!(
            count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
            0
        );
    }
    fs::write(
        root.path().join("app.ts"),
        "import {run} from './lib.js'; export function caller() { run(); }",
    )
    .unwrap();
    let result = sync(root.path(), "app.ts");
    assert_eq!(result["globalFreshnessCertified"], false);
    // View is the index, not the OS: lib.js cannot bypass publication/hash checks.
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib.ts'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM files WHERE path='lib.js'"
        ),
        0
    );
}

#[test]
fn semantic_import_bindings_resolve_only_the_unshadowed_call_site() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("lib.ts"), "export function run() {}").unwrap();
    sync(root.path(), "lib.ts");
    fs::write(root.path().join("app.ts"), "import {run as execute} from './lib';\nfunction local(execute: () => void) { execute(); }\nexport function caller() { execute(); register(execute); }\n").unwrap();
    sync(root.path(), "app.ts");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib.ts' AND s.name='caller'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib.ts' AND s.name='local'"
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs r JOIN nodes s ON s.id=r.from_node_id WHERE r.reference_name='execute' AND r.reference_kind='calls' AND s.name='local'"
        ),
        1
    );
    // Updating the target rechecks semantic sites, not just the imported name.
    fs::write(root.path().join("lib.ts"), "\nexport function run() {}").unwrap();
    sync(root.path(), "lib.ts");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib.ts'"
        ),
        1
    );
}

#[test]
fn semantic_block_shadowing_and_utf16_positions_are_used_in_publication() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("lib.js"), "export function run() {}").unwrap();
    sync(root.path(), "lib.js");
    fs::write(root.path().join("app.js"), "\u{feff}import {run as execute} from './lib.js';\r\nexport function caller() { const text = '😀'; { const execute = () => {}; execute(); } execute(); }\r\n").unwrap();
    sync(root.path(), "app.js");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib.js'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_name='execute' AND reference_kind='calls'"
        ),
        1
    );
    // Replace with parameter shadowing: the former imported edge must disappear.
    fs::write(
        root.path().join("app.js"),
        "import {run as execute} from './lib.js'; export function caller(execute) { execute(); }",
    )
    .unwrap();
    sync(root.path(), "app.js");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib.js'"
        ),
        0
    );
}

#[test]
fn semantic_conflicting_binding_rolls_back_previous_publication() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("lib.ts"), "export function run() {}").unwrap();
    sync(root.path(), "lib.ts");
    fs::write(
        root.path().join("app.ts"),
        "import {run as execute} from './lib'; export function caller() { execute(); }",
    )
    .unwrap();
    sync(root.path(), "app.ts");
    let before: String = db(root.path())
        .query_row(
            "SELECT content_hash FROM files WHERE path='app.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    fs::write(root.path().join("app.ts"), "import {run as execute} from './lib'; const execute = () => {}; export function caller() { execute(); }").unwrap();
    assert_eq!(
        sync_file(root.path(), "app.ts", &OperationContext::default())
            .unwrap_err()
            .0,
        "SYNC_SEMANTIC"
    );
    let after: String = db(root.path())
        .query_row(
            "SELECT content_hash FROM files WHERE path='app.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
}

#[test]
fn named_import_alias_uses_module_not_global_homonyms() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    for (path, source) in [
        ("lib.ts", "export function run() {}"),
        (
            "other.ts",
            "export function run() {} export function execute() {}",
        ),
        (
            "app.ts",
            "import {run as execute} from './lib'; export function caller() { execute(); }",
        ),
    ] {
        fs::write(root.path().join(path), source).unwrap();
        sync(root.path(), path);
    }
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='lib.ts' AND t.name='run' AND json_extract(e.metadata,'$.refName')='execute' AND json_extract(e.metadata,'$.resolvedBy')='native-relative-named-import-subset'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='other.ts'"
        ),
        0
    );
}

#[test]
fn unsupported_imports_and_shadowed_aliases_never_fall_back_to_globals() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("lib.ts"),
        "export function run() {} export function execute() {}",
    )
    .unwrap();
    sync(root.path(), "lib.ts");
    for source in [
        "import {run} from 'external'; export function caller() { run(); }",
        "import run from './lib'; export function caller() { run(); }",
        "import * as ns from './lib'; export function caller() { ns.run(); }",
        "import type {run} from './lib'; export function caller() { run(); }",
        "import {run as execute} from './lib'; export function caller(execute: () => void) { execute(); }",
        "import {run as execute} from './lib'; export function caller() { const execute = () => {}; execute(); }",
        "import {run} from './missing'; export function caller() { run(); }",
        "import {run} from '../../lib'; export function caller() { run(); }",
    ] {
        fs::write(root.path().join("app.ts"), source).unwrap();
        sync(root.path(), "app.ts");
        assert_eq!(
            count(
                root.path(),
                "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='app.ts' AND t.file_path='lib.ts'"
            ),
            0,
            "{source}"
        );
    }
}

#[test]
fn late_import_target_retries_alias_and_deletion_recovers_original_name() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("app.js"),
        "import {run as execute} from './lib.js'; export function caller() { execute(); }",
    )
    .unwrap();
    sync(root.path(), "app.js");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        0
    );
    fs::write(root.path().join("lib.js"), "export function run() {}").unwrap();
    sync(root.path(), "lib.js");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
    fs::remove_file(root.path().join("lib.js")).unwrap();
    sync(root.path(), "lib.js");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_name='execute' AND reference_kind='calls'"
        ),
        1
    );
    fs::write(root.path().join("lib.js"), "export function run() {}").unwrap();
    sync(root.path(), "lib.js");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
}

#[test]
fn provider_module_priority_is_respected_and_nonexported_targets_remain_pending() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir(root.path().join("lib")).unwrap();
    for path in ["lib.ts", "lib/index.ts"] {
        fs::write(root.path().join(path), "export function run() {}").unwrap();
        sync(root.path(), path);
    }
    fs::write(
        root.path().join("app.ts"),
        "import {run} from './lib'; export function caller() { run(); }",
    )
    .unwrap();
    sync(root.path(), "app.ts");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
    fs::write(root.path().join("lib.ts"), "function run() {}").unwrap();
    sync(root.path(), "lib.ts");
    fs::write(
        root.path().join("app.ts"),
        "import {run} from './lib.ts'; export function caller() { run(); }",
    )
    .unwrap();
    sync(root.path(), "app.ts");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        0
    );
}

#[test]
fn prior_import_edges_are_rechecked_when_exports_or_module_candidates_change() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir(root.path().join("lib")).unwrap();
    fs::write(root.path().join("lib.ts"), "export function run() {}").unwrap();
    sync(root.path(), "lib.ts");
    fs::write(
        root.path().join("app.ts"),
        "import {run as execute} from './lib'; export function caller() { execute(); }",
    )
    .unwrap();
    sync(root.path(), "app.ts");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
    fs::write(root.path().join("lib.ts"), "function run() {}").unwrap();
    assert_eq!(sync(root.path(), "lib.ts")["importReferencesRechecked"], 1);
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        0
    );
    fs::write(root.path().join("lib.ts"), "export function run() {}").unwrap();
    sync(root.path(), "lib.ts");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
    fs::write(root.path().join("lib/index.ts"), "export function run() {}").unwrap();
    sync(root.path(), "lib/index.ts");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
    fs::remove_file(root.path().join("lib/index.ts")).unwrap();
    sync(root.path(), "lib/index.ts");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
    fs::write(
        root.path().join("lib.ts"),
        "export default function run() {}",
    )
    .unwrap();
    sync(root.path(), "lib.ts");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        0
    );
}

#[test]
fn cli_resolves_relative_import_alias_with_no_node_on_path() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("lib.ts"), "export function run() {}").unwrap();
    sync(root.path(), "lib.ts");
    fs::write(
        root.path().join("src/app.ts"),
        "import {run as execute} from '../lib.js'; function local(execute: () => void) { execute(); } export function caller() { execute(); }",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agent-harness"))
        .env("PATH", "")
        .args(["--root"])
        .arg(root.path())
        .args(["--sync-file", "src/app.ts"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
}

#[test]
#[ignore = "requires reference CodeGraph package and Node; compares import mappings/paths only"]
fn import_mapping_and_unambiguous_paths_match_provider_subset() {
    let package = std::env::var("HARNESS_CODEGRAPH_PACKAGE").expect("provider package");
    let node = std::env::var("HARNESS_CODEGRAPH_NODE").expect("reference Node");
    let script = r#"
const path = require('path');
const api = require(path.join(process.argv[1], 'dist/resolution/import-resolver.js'));
const input = JSON.parse(process.argv[2]);
const ctx = { getProjectRoot: () => process.cwd(), fileExists: p => input.files.includes(p) };
const mappings = api.extractImportMappings(input.from, input.signature, 'typescript');
process.stdout.write(JSON.stringify({mappings, target: api.resolveImportPath(input.module, input.from, 'typescript', ctx)}));
"#;
    for (from, signature, files, expected) in [
        (
            "app.ts",
            "import {run as execute} from './lib';",
            vec!["lib.ts"],
            "lib.ts",
        ),
        (
            "src/app.ts",
            "import {run} from '../lib.js';",
            vec!["lib.js"],
            "lib.js",
        ),
        (
            "src/app.ts",
            "import {run as execute, other} from './lib';",
            vec!["src/lib/index.ts"],
            "src/lib/index.ts",
        ),
    ] {
        let (module, names) = codegraph_native::named_imports(signature).unwrap();
        let input =
            serde_json::json!({"from":from,"signature":signature,"module":module,"files":files});
        let output = Command::new(&node)
            .args(["-e", script, &package, &input.to_string()])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let provider: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(provider["target"], expected);
        let mappings = names.into_iter().map(|(original, local)| serde_json::json!({"localName":local,"exportedName":original,"source":module,"isDefault":false,"isNamespace":false})).collect::<Vec<_>>();
        assert_eq!(provider["mappings"], serde_json::json!(mappings));
    }
}

#[test]
fn native_writer_resolves_cross_file_calls_preserves_line_shifts_and_recovers_renames() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/lib.rs"), "mod a; mod b; mod c;").unwrap();
    sync(root.path(), "src/lib.rs");
    fs::write(root.path().join("src/a.rs"), "pub fn target() {}\n").unwrap();
    assert_eq!(sync(root.path(), "src/a.rs")["committed"], true);
    fs::write(
        root.path().join("src/b.rs"),
        "pub fn caller() { super::a::target(); }\n",
    )
    .unwrap();
    let result = sync(root.path(), "src/b.rs");
    assert_eq!(result["referencesResolved"], 1);
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE is_exported IS NULL OR is_async IS NULL OR is_static IS NULL OR is_abstract IS NULL"
        ),
        0
    );
    let before: String = db(root.path())
        .query_row("SELECT id FROM nodes WHERE name='target'", [], |r| r.get(0))
        .unwrap();
    fs::write(root.path().join("src/a.rs"), "\n\npub fn target() {}\n").unwrap();
    let shifted = sync(root.path(), "src/a.rs");
    assert_eq!(shifted["incomingEdgesRetained"], 1);
    let after: String = db(root.path())
        .query_row("SELECT id FROM nodes WHERE name='target'", [], |r| r.get(0))
        .unwrap();
    assert_ne!(before, after);
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
    fs::write(root.path().join("src/a.rs"), "pub fn renamed() {}\n").unwrap();
    let renamed = sync(root.path(), "src/a.rs");
    assert_eq!(renamed["incomingReferencesRecovered"], 1);
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_name='super::a::target'"
        ),
        1
    );
    fs::write(root.path().join("src/c.rs"), "pub fn target() {}\n").unwrap();
    // A homonym in another module cannot satisfy super::a::target.
    assert_eq!(sync(root.path(), "src/c.rs")["referencesResolved"], 0);
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        0
    );
    fs::remove_file(root.path().join("src/c.rs")).unwrap();
    let deleted = sync(root.path(), "src/c.rs");
    assert_eq!(deleted["deleted"], true);
    assert_eq!(deleted["synchronized"], false);
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM files WHERE path='src/c.rs'"
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_name='super::a::target'"
        ),
        1
    );
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM pragma_foreign_key_check"),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes_fts WHERE nodes_fts MATCH 'renamed'"
        ),
        1
    );
}

#[test]
fn ambiguous_calls_and_receiver_qualifiers_are_kept_unresolved() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    for path in ["a.rs", "b.rs"] {
        fs::write(root.path().join(path), "pub fn target() {}\n").unwrap();
        sync(root.path(), path);
    }
    fs::write(
        root.path().join("c.rs"),
        "pub fn caller() { target(); object.run(); }\n",
    )
    .unwrap();
    let result = sync(root.path(), "c.rs");
    assert_eq!(result["referencesResolved"], 0);
    assert!(result["referencesPending"].as_u64().unwrap() >= 2);
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        0
    );
}

#[test]
fn legacy_unrecoverable_incoming_edges_block_deletion_without_data_loss() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/lib.rs"), "mod a; mod b;\n").unwrap();
    sync(root.path(), "src/lib.rs");
    fs::write(root.path().join("src/a.rs"), "pub fn target() {}\n").unwrap();
    sync(root.path(), "src/a.rs");
    fs::write(
        root.path().join("src/b.rs"),
        "pub fn caller() { super::a::target(); }\n",
    )
    .unwrap();
    sync(root.path(), "src/b.rs");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1,
        "the fixture must establish a proven cross-module call before removing its recovery metadata"
    );
    db(root.path())
        .execute("UPDATE edges SET metadata=NULL WHERE kind='calls'", [])
        .unwrap();
    fs::remove_file(root.path().join("src/a.rs")).unwrap();
    let error = sync_file(root.path(), "src/a.rs", &OperationContext::default()).unwrap_err();
    assert_eq!(error.0, "SYNC_LEGACY_REFERENCE");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE name='target'"
        ),
        1
    );
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
}

#[test]
fn constraint_failure_after_deletion_rolls_back_nodes_files_and_fts() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("a.rs"), "pub fn old() {}\n").unwrap();
    sync(root.path(), "a.rs");
    let digest: String = db(root.path())
        .query_row(
            "SELECT content_hash FROM files WHERE path='a.rs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db(root.path())
        .execute("CREATE UNIQUE INDEX test_one_kind ON nodes(kind)", [])
        .unwrap();
    fs::write(
        root.path().join("a.rs"),
        "pub fn first() {}\npub fn second() {}\n",
    )
    .unwrap();
    assert!(sync_file(root.path(), "a.rs", &OperationContext::default()).is_err());
    let stored: String = db(root.path())
        .query_row(
            "SELECT content_hash FROM files WHERE path='a.rs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, digest);
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM nodes WHERE name='old'"),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes_fts WHERE nodes_fts MATCH 'old'"
        ),
        1
    );
}

#[test]
fn rejects_unknown_triggers_bad_sources_cancellation_and_busy_index() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("a.rs"), "pub fn old() {}\n").unwrap();
    sync(root.path(), "a.rs");
    let cancelled = OperationContext::default();
    cancelled.cancel();
    assert_eq!(
        sync_file(root.path(), "a.rs", &cancelled).unwrap_err().0,
        "CANCELLED"
    );
    fs::write(root.path().join("a.rs"), "fn broken( {").unwrap();
    assert_eq!(
        sync_file(root.path(), "a.rs", &OperationContext::default())
            .unwrap_err()
            .0,
        "EXTRACTION_DEFERRED"
    );
    fs::write(root.path().join("a.rs"), "pub fn changed() {}\n").unwrap();
    let lock = db(root.path());
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(sync_file(root.path(), "a.rs", &OperationContext::default()).is_err());
    lock.execute_batch("ROLLBACK").unwrap();
    drop(lock);
    db(root.path())
        .execute_batch(
            "CREATE TRIGGER malicious AFTER INSERT ON nodes BEGIN DELETE FROM files; END;",
        )
        .unwrap();
    assert_eq!(
        sync_file(root.path(), "a.rs", &OperationContext::default())
            .unwrap_err()
            .0,
        "SYNC_SCHEMA"
    );
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM nodes WHERE name='old'"),
        1
    );
}

#[test]
fn invalidates_provider_global_stamp_and_updates_real_file_hash() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    let connection = db(root.path());
    for key in ["indexed_at_commit", "indexed_dirty_paths"] {
        connection
            .execute("INSERT INTO project_metadata VALUES(?1,'old',0)", [key])
            .unwrap();
    }
    drop(connection);
    let source = "namespace Demo; public class Item { public void Run() { Helper(); } public void Helper() {} }\n";
    fs::write(root.path().join("Item.cs"), source).unwrap();
    let result = sync(root.path(), "Item.cs");
    assert_eq!(result["referencesResolved"], 1);
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM project_metadata WHERE key='indexed_at_commit'"
        ),
        0
    );
    let digest: String = db(root.path())
        .query_row(
            "SELECT content_hash FROM files WHERE path='Item.cs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(digest, harness_domain::hash(source.as_bytes()));
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes_fts WHERE nodes_fts MATCH 'Helper'"
        ),
        1
    );
}

#[test]
fn cli_sync_file_publishes_without_node_or_git() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("a.ts"),
        "export function first() {}\nexport function second() { first(); }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agent-harness"))
        .env("PATH", "")
        .arg("--root")
        .arg(root.path())
        .args(["--sync-file", "a.ts"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["committed"], true);
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
}

#[test]
fn rejects_hardlinked_database_and_sidecars() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("a.rs"), "pub fn f() {}\n").unwrap();
    fs::hard_link(
        root.path().join(".codegraph/codegraph.db"),
        root.path().join("alias.db"),
    )
    .unwrap();
    assert_eq!(
        sync_file(root.path(), "a.rs", &OperationContext::default())
            .unwrap_err()
            .0,
        "PATH"
    );
    assert_eq!(count(root.path(), "SELECT count(*) FROM nodes"), 0);
}

#[test]
fn unchanged_sources_do_not_clear_provider_stamps_or_rebuild_graphs() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("a.rs"), "pub fn f() {}\n").unwrap();
    sync(root.path(), "a.rs");
    db(root.path())
        .execute(
            "INSERT INTO project_metadata VALUES('indexed_at_commit','provider-stamp',0)",
            [],
        )
        .unwrap();
    let result = sync(root.path(), "a.rs");
    assert_eq!(result["changed"], false);
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM project_metadata WHERE key='indexed_at_commit'"
        ),
        1
    );
}

#[test]
fn synthesized_graph_edges_block_unsupported_rewrites() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("a.rs"),
        "pub fn first() {}\npub fn second() {}\n",
    )
    .unwrap();
    sync(root.path(), "a.rs");
    db(root.path()).execute_batch("INSERT INTO edges(source,target,kind,metadata) SELECT a.id,b.id,'calls','{\"synthesizedBy\":\"framework\"}' FROM nodes a,nodes b WHERE a.name='first' AND b.name='second'").unwrap();
    fs::write(root.path().join("a.rs"), "pub fn renamed() {}\n").unwrap();
    assert_eq!(
        sync_file(root.path(), "a.rs", &OperationContext::default())
            .unwrap_err()
            .0,
        "SYNC_UNSUPPORTED_RELATION"
    );
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM nodes WHERE name='first'"),
        1
    );
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
}

#[test]
fn hardlinked_wal_is_rejected_before_sqlite_opens_it() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("a.rs"), "pub fn f() {}\n").unwrap();
    let wal = root.path().join(".codegraph/codegraph.db-wal");
    fs::write(&wal, []).unwrap();
    fs::hard_link(&wal, root.path().join("linked-wal")).unwrap();
    assert_eq!(
        sync_file(root.path(), "a.rs", &OperationContext::default())
            .unwrap_err()
            .0,
        "PATH"
    );
    assert_eq!(
        fs::metadata(root.path().join("linked-wal")).unwrap().len(),
        0
    );
}

#[test]
fn third_file_wiring_and_recorded_synthesis_inputs_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    for (path, name) in [("a.rs", "first"), ("b.rs", "second"), ("wiring.rs", "wire")] {
        fs::write(root.path().join(path), format!("pub fn {name}() {{}}\n")).unwrap();
        sync(root.path(), path);
    }
    db(root.path()).execute_batch("INSERT INTO edges(source,target,kind,metadata) SELECT a.id,b.id,'calls','{\"synthesizedBy\":\"callback\",\"registeredAt\":\"wiring.rs:1\"}' FROM nodes a,nodes b WHERE a.name='first' AND b.name='second'").unwrap();
    fs::write(root.path().join("wiring.rs"), "pub fn changed() {}\n").unwrap();
    assert_eq!(
        sync_file(root.path(), "wiring.rs", &OperationContext::default())
            .unwrap_err()
            .0,
        "SYNC_UNSUPPORTED_RELATION"
    );
    let connection = db(root.path());
    connection
        .execute("DELETE FROM edges WHERE kind='calls'", [])
        .unwrap();
    connection
        .execute("INSERT INTO synthesis_inputs VALUES('wiring.rs')", [])
        .unwrap();
    drop(connection);
    assert_eq!(
        sync_file(root.path(), "wiring.rs", &OperationContext::default())
            .unwrap_err()
            .0,
        "SYNC_UNSUPPORTED_RELATION"
    );
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM nodes WHERE name='wire'"),
        1
    );
}

#[test]
fn cross_file_private_symbols_are_not_bound_by_bare_name() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("a.rs"), "fn hidden() {}\n").unwrap();
    sync(root.path(), "a.rs");
    fs::write(root.path().join("b.rs"), "pub fn caller() { hidden(); }\n").unwrap();
    let result = sync(root.path(), "b.rs");
    assert_eq!(result["referencesResolved"], 0);
    assert_eq!(result["referencesPending"], 1);
}

#[test]
fn missing_index_and_non_wal_journal_do_not_create_or_modify_graphs() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.rs"), "pub fn f() {}\n").unwrap();
    assert!(sync_file(root.path(), "a.rs", &OperationContext::default()).is_err());
    assert!(!root.path().join(".codegraph").exists());
    fixture(root.path());
    db(root.path())
        .execute_batch("PRAGMA journal_mode=DELETE")
        .unwrap();
    assert_eq!(
        sync_file(root.path(), "a.rs", &OperationContext::default())
            .unwrap_err()
            .0,
        "SYNC_JOURNAL"
    );
    assert_eq!(count(root.path(), "SELECT count(*) FROM nodes"), 0);
}

#[test]
fn workspace_sync_applies_native_additions_changes_and_explicit_deletions() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/lib.rs"), "mod a; mod b;\n").unwrap();
    fs::write(root.path().join("src/a.rs"), "pub fn target() {}\n").unwrap();
    fs::write(
        root.path().join("src/b.rs"),
        "pub fn caller() { super::a::target(); }\n",
    )
    .unwrap();
    let result = sync_workspace(root.path(), &OperationContext::default()).unwrap();
    assert_eq!(result["changedFilesCommitted"], 3);
    assert_eq!(result["selectedScopeApplied"], true);
    assert_eq!(result["synchronized"], false);
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
    fs::remove_file(root.path().join("src/b.rs")).unwrap();
    fs::write(root.path().join("src/a.rs"), "pub fn modified() {}\n").unwrap();
    let result = sync_workspace(root.path(), &OperationContext::default()).unwrap();
    assert_eq!(result["changedFilesCommitted"], 2);
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM files WHERE path='src/b.rs'"
        ),
        0
    );
}

#[test]
fn batch_failure_reports_prior_commits_instead_of_claiming_global_rollback() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("a.rs"), "pub fn valid() {}\n").unwrap();
    fs::write(root.path().join("b.rs"), "fn broken( {").unwrap();
    let result = sync_workspace(root.path(), &OperationContext::default()).unwrap();
    assert_eq!(result["changedFilesCommitted"], 1);
    assert_eq!(result["selectedScopeApplied"], false);
    assert_eq!(result["failure"]["code"], "EXTRACTION_DEFERRED");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM nodes WHERE name='valid'"),
        1
    );
    let output = Command::new(env!("CARGO_BIN_EXE_agent-harness"))
        .env("PATH", "")
        .arg("--root")
        .arg(root.path())
        .arg("--sync")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["failure"]["code"], "EXTRACTION_DEFERRED");
}

#[test]
fn automatic_scope_refuses_unported_provider_configuration_before_any_commit() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("a.rs"), "pub fn f() {}\n").unwrap();
    fs::write(
        root.path().join("codegraph.json"),
        "{\"extensions\":{\".custom\":\"rust\"}}",
    )
    .unwrap();
    assert_eq!(
        sync_workspace(root.path(), &OperationContext::default())
            .unwrap_err()
            .0,
        "SYNC_SCOPE"
    );
    assert_eq!(count(root.path(), "SELECT count(*) FROM nodes"), 0);
}

#[test]
fn bounded_batches_report_remaining_files_and_replan_on_next_run() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    for i in 0..33 {
        fs::write(
            root.path().join(format!("f{i:02}.rs")),
            format!("pub fn f{i}() {{}}\n"),
        )
        .unwrap();
    }
    let first = sync_workspace(root.path(), &OperationContext::default()).unwrap();
    assert_eq!(first["changedFilesCommitted"], 32);
    assert_eq!(first["nativeFilesRemaining"], 1);
    assert_eq!(first["selectedScopeApplied"], false);
    let second = sync_workspace(root.path(), &OperationContext::default()).unwrap();
    assert_eq!(second["changedFilesCommitted"], 1);
    assert_eq!(second["selectedScopeApplied"], true);
    assert_eq!(count(root.path(), "SELECT count(*) FROM files"), 33);
}

#[test]
fn missing_fts_triggers_are_rejected_instead_of_silently_staling_search() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("a.rs"), "pub fn old() {}\n").unwrap();
    sync(root.path(), "a.rs");
    db(root.path())
        .execute_batch("DROP TRIGGER nodes_ad")
        .unwrap();
    fs::write(root.path().join("a.rs"), "pub fn changed() {}\n").unwrap();
    assert_eq!(
        sync_file(root.path(), "a.rs", &OperationContext::default())
            .unwrap_err()
            .0,
        "SYNC_SCHEMA"
    );
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM nodes WHERE name='old'"),
        1
    );
}
