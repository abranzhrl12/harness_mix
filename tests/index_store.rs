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
fn go_native_graph_publishes_declarations_imports_and_is_explorable() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("main.go"), "package demo\nimport \"fmt\"\ntype Box struct { Name string }\nfunc (b *Box) Run() { fmt.Println(b.Name) }\n").unwrap();
    let result = sync(root.path(), "main.go");
    assert_eq!(result["committed"], true, "{result}");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM files WHERE language='go'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE file_path='main.go' AND kind='struct' AND name='Box'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE file_path='main.go' AND kind='method' AND name='Run'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE file_path='main.go' AND kind='import'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE kind='imports'"
        ),
        1
    );
    let app = agent_harness::adapters::application(root.path()).unwrap();
    let request = serde_json::from_value(
        serde_json::json!({"filePath":"main.go","symbol":"Run","view":"summary"}),
    )
    .unwrap();
    let explored = app.execute(harness_domain::contracts::Request::Explore(request));
    assert_eq!(explored["state"], "ok", "{explored}");
}

#[test]
fn razor_publication_resolves_members_and_model_and_updates_atomically() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("Model.cs"),
        "namespace Demo; public class Model {}",
    )
    .unwrap();
    sync(root.path(), "Model.cs");
    let source = "@model Demo.Model\n@section Scripts { <script>fake();</script> }\n@functions { public void Run() { Helper(); } public void Helper() {} }\n";
    fs::write(root.path().join("Index.cshtml"), source).unwrap();
    let first = sync(root.path(), "Index.cshtml");
    assert_eq!(first["committed"], true, "{first}");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM files WHERE language='razor'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.name='Run' AND t.name='Helper'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='references' AND t.file_path='Model.cs'"
        ),
        1
    );
    let app = agent_harness::adapters::application(root.path()).unwrap();
    let request = serde_json::from_value(
        serde_json::json!({"query":"Run","filePath":"Index.cshtml","symbol":"Run","view":"full"}),
    )
    .unwrap();
    let explored = app.execute(harness_domain::contracts::Request::Explore(request));
    assert_eq!(explored["state"], "ok", "{explored}");
    let unchanged = sync(root.path(), "Index.cshtml");
    assert_eq!(unchanged["extractionPerformed"], false);
    fs::write(
        root.path().join("Index.cshtml"),
        "@functions { public void Run( {} }",
    )
    .unwrap();
    assert!(sync_file(root.path(), "Index.cshtml", &OperationContext::default()).is_err());
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE name='Helper'"
        ),
        1
    );
    fs::write(
        root.path().join("Index.cshtml"),
        "@functions { public void Changed() {} }",
    )
    .unwrap();
    sync(root.path(), "Index.cshtml");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE name='Helper'"
        ),
        0
    );
    fs::remove_file(root.path().join("Index.cshtml")).unwrap();
    sync(root.path(), "Index.cshtml");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE language='razor'"
        ),
        0
    );
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM pragma_foreign_key_check"),
        0
    );
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
fn semantic_conflicting_binding_commits_partial_without_a_false_import_edge() {
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
    let prior_import_edge = "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.name='caller' AND t.file_path='lib.ts'";
    assert_eq!(count(root.path(), prior_import_edge), 1);

    let changed_source = "import {run as execute} from './lib'; const execute = () => {}; export function caller() { execute(); }";
    fs::write(root.path().join("app.ts"), changed_source).unwrap();
    let report = sync_file(root.path(), "app.ts", &OperationContext::default()).unwrap();
    assert_eq!(report["committed"], true, "{report}");
    assert_eq!(report["graphStatus"], "partial", "{report}");
    assert_eq!(report["warningFiles"], 1, "{report}");
    assert_eq!(report["warnings"][0]["path"], "app.ts");
    assert_eq!(report["warnings"][0]["code"], "SYNC_SEMANTIC");
    assert_eq!(report["warnings"][0]["severity"], "warning");
    assert_eq!(report["warnings"][0]["unresolvedReferencesRetained"], true);

    let after: String = db(root.path())
        .query_row(
            "SELECT content_hash FROM files WHERE path='app.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(before, after);
    assert_eq!(after, harness_domain::hash(changed_source.as_bytes()));
    assert_eq!(
        count(root.path(), prior_import_edge),
        0,
        "an ambiguous import must not be published as a resolved call"
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs r JOIN nodes s ON s.id=r.from_node_id WHERE r.reference_name='execute' AND r.reference_kind='calls' AND s.name='caller'"
        ),
        1,
        "the ambiguous call should remain pending"
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
fn synthesized_graph_edges_are_archived_without_blocking_source_updates() {
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
    let published = sync(root.path(), "a.rs");
    assert_eq!(published["committed"], true, "{published}");
    assert!(published.get("graphStatus").is_none(), "{published}");
    assert_eq!(published["indexArchive"]["obsolete"], 1);
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM nodes WHERE name='first'"),
        0
    );
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE name='renamed'"
        ),
        1
    );
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM native_invalidated_edges"),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM native_invalidated_edges WHERE json_extract(snapshot,'$.metadata') LIKE '%framework%' AND json_extract(snapshot,'$.sourceHash') IS NOT NULL"
        ),
        1
    );
    let unchanged = sync(root.path(), "a.rs");
    assert_eq!(unchanged["changed"], false);
    assert!(unchanged.get("graphStatus").is_none());
    assert_eq!(unchanged["indexArchive"]["obsolete"], 1);
    for _ in 0..2 {
        let rebuilt = agent_harness::adapters::index_store::rebuild_workspace(
            root.path(),
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(rebuilt["providerDependentFiles"], 0, "{rebuilt}");
        assert_eq!(rebuilt["failedFiles"], 0, "{rebuilt}");
        assert_eq!(rebuilt["sourceScopeApplied"], true, "{rebuilt}");
        assert_eq!(rebuilt["selectedScopeApplied"], true, "{rebuilt}");
    }
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM native_invalidated_edges"),
        1
    );
    let app = agent_harness::adapters::application(root.path()).unwrap();
    let result = app.execute(harness_domain::contracts::Request::Explore(serde_json::from_value(
        serde_json::json!({"symbol":"renamed","filePath":"a.rs","view":"summary","syncIndex":false})
    ).unwrap()));
    assert!(result["data"].get("graphStatus").is_none(), "{result}");
    let synchronized = app.execute(harness_domain::contracts::Request::Explore(serde_json::from_value(
        serde_json::json!({"symbol":"renamed","filePath":"a.rs","view":"summary","syncIndex":true})
    ).unwrap()));
    assert_eq!(
        synchronized["data"]["focus"]["status"], "resolved",
        "{synchronized}"
    );
    assert!(synchronized["data"].get("graphStatus").is_none());
    let output = Command::new(env!("CARGO_BIN_EXE_agent-harness"))
        .arg("--root")
        .arg(root.path())
        .args(["--index", "rebuild"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["sourceScopeApplied"], true);
    assert_eq!(report["selectedScopeApplied"], true);
    assert_eq!(report["warningFiles"], 0);
    // A cleaned-up legacy index and a new index must agree on the active graph;
    // keeping audit history must not create perpetual coverage debt.
    let clean = tempfile::tempdir().unwrap();
    fixture(clean.path());
    fs::write(clean.path().join("a.rs"), "pub fn renamed() {}\n").unwrap();
    sync(clean.path(), "a.rs");
    fn canonical(root: &Path) -> Vec<(String, String, String)> {
        let db = db(root);
        let mut statement = db.prepare("SELECT file_path,kind,qualified_name FROM nodes
            UNION ALL SELECT s.file_path || ':' || s.qualified_name,e.kind,t.file_path || ':' || t.qualified_name
            FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target
            ORDER BY 1,2,3").unwrap();
        statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }
    assert_eq!(canonical(root.path()), canonical(clean.path()));
}

#[test]
fn pending_archive_is_scoped_and_survives_a_no_change_workspace_sync() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("a.rs"),
        "pub fn first() {}\npub fn second() {}\n",
    )
    .unwrap();
    fs::write(root.path().join("other.rs"), "pub fn unrelated() {}\n").unwrap();
    sync(root.path(), "a.rs");
    sync(root.path(), "other.rs");
    db(root.path()).execute_batch("INSERT INTO edges(source,target,kind,metadata) SELECT a.id,b.id,'calls','{\"synthesizedBy\":\"framework\"}' FROM nodes a,nodes b WHERE a.name='first' AND b.name='second'").unwrap();
    let published = agent_harness::adapters::index_store::sync_file_forced(
        root.path(),
        "a.rs",
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(published["indexArchive"]["pending"], 1);
    let no_change = agent_harness::adapters::index_store::sync_workspace_all(
        root.path(),
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(no_change["sourceScopeApplied"], true);
    assert_eq!(no_change["selectedScopeApplied"], false, "{no_change}");
    assert_eq!(no_change["indexArchive"]["pending"], 1);
    let app = agent_harness::adapters::application(root.path()).unwrap();
    for (file, name, partial) in [("a.rs", "first", true), ("other.rs", "unrelated", false)] {
        let result = app.execute(harness_domain::contracts::Request::Explore(serde_json::from_value(
            serde_json::json!({"symbol":name,"filePath":file,"view":"summary","syncIndex":false})
        ).unwrap()));
        assert_eq!(
            result["data"]["graphStatus"] == "partial",
            partial,
            "{result}"
        );
    }
}

#[test]
fn resolver_upgrade_refreshes_unchanged_sources_once_without_rebuild() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("a.rs"),
        "pub fn first() {}\npub fn caller() { first(); }\n",
    )
    .unwrap();
    sync(root.path(), "a.rs");
    // Simulate an index produced by the previous native resolver. The source
    // hash remains identical; a normal authorized sync must refresh its proofs.
    db(root.path())
        .execute_batch(
            "DELETE FROM project_metadata WHERE key LIKE 'native_rust_proof:%';
        DELETE FROM edges WHERE kind='calls';",
        )
        .unwrap();
    let upgraded = agent_harness::adapters::index_store::sync_workspace_all(
        root.path(),
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(upgraded["changedFilesCommitted"], 1, "{upgraded}");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        1
    );
    let unchanged = agent_harness::adapters::index_store::sync_workspace_all(
        root.path(),
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(unchanged["changedFilesCommitted"], 0, "{unchanged}");
    assert_eq!(unchanged["failedFiles"], 0);
}

#[test]
fn local_trait_implementation_replaces_legacy_calls_without_losing_audit_history() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    let source = "trait Render { fn draw(&self); }\nstruct Card;\nimpl Render for Card { fn draw(&self) {} }\n";
    fs::write(root.path().join("a.rs"), source).unwrap();
    sync(root.path(), "a.rs");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE kind='implements' AND json_extract(metadata,'$.resolvedBy')='rust-local-trait-impl-v1'"
        ),
        1
    );
    db(root.path()).execute_batch("UPDATE edges SET kind='calls',metadata='{\"synthesizedBy\":\"interface-impl\"}' WHERE kind='implements' AND json_extract(metadata,'$.resolvedBy')='rust-local-trait-impl-v1'").unwrap();
    let restored = agent_harness::adapters::index_store::sync_file_forced(
        root.path(),
        "a.rs",
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(restored["indexArchive"]["superseded"], 1, "{restored}");
    assert_eq!(restored["indexArchive"]["pending"], 0);
    assert!(restored.get("graphStatus").is_none());
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        0
    );
    for _ in 0..2 {
        let report = agent_harness::adapters::index_store::rebuild_workspace(
            root.path(),
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(report["selectedScopeApplied"], true, "{report}");
        assert_eq!(
            count(
                root.path(),
                "SELECT count(*) FROM edges WHERE kind='implements' AND json_extract(metadata,'$.resolvedBy')='rust-local-trait-impl-v1'"
            ),
            1
        );
    }
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM native_invalidated_edges"),
        1
    );
    let clean = tempfile::tempdir().unwrap();
    fixture(clean.path());
    fs::write(clean.path().join("a.rs"), source).unwrap();
    sync(clean.path(), "a.rs");
    let query = "SELECT s.qualified_name,e.kind,t.qualified_name FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target ORDER BY 1,2,3";
    let relationships = |path: &Path| {
        let connection = db(path);
        let mut statement = connection.prepare(query).unwrap();
        statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(relationships(root.path()), relationships(clean.path()));
}

#[test]
fn unrelated_import_and_expression_macro_do_not_hide_a_local_call() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("a.rs"), "use std::collections::BTreeMap;\nfn explore_inner() {}\nfn explore() { let _payload = serde_json::json!({\"ok\":true}); explore_inner(); }\n").unwrap();
    sync(root.path(), "a.rs");
    let query = "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.name='explore' AND t.name='explore_inner'";
    assert_eq!(count(root.path(), query), 1);
    for _ in 0..2 {
        agent_harness::adapters::index_store::rebuild_workspace(
            root.path(),
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(count(root.path(), query), 1);
    }
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
    let updated = sync(root.path(), "wiring.rs");
    assert_eq!(updated["committed"], true, "{updated}");
    assert_eq!(updated["warnings"][0]["pendingRelations"], 1);
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        0
    );
    let connection = db(root.path());
    connection
        .execute("DELETE FROM edges WHERE kind='calls'", [])
        .unwrap();
    connection
        .execute("INSERT INTO synthesis_inputs VALUES('wiring.rs')", [])
        .unwrap();
    drop(connection);
    fs::write(root.path().join("wiring.rs"), "pub fn changed_again() {}\n").unwrap();
    // A recorded synthesis input can be published natively: the recorded input
    // and the provider data survive the publication.
    let published = sync_file(root.path(), "wiring.rs", &OperationContext::default()).unwrap();
    assert_eq!(published["committed"], true, "{published}");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE name='changed_again'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM synthesis_inputs WHERE file_path='wiring.rs'"
        ),
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

#[test]
fn windows_separators_are_normalized_before_path_validation() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir_all(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/a.rs"), "pub fn first() {}\n").unwrap();
    let result = sync_file(root.path(), "src\\a.rs", &OperationContext::default()).unwrap();
    assert_eq!(result["committed"], true, "{result}");
    assert_eq!(result["path"], "src/a.rs");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM files WHERE path='src/a.rs'"
        ),
        1
    );
}

#[test]
fn provider_framework_nodes_survive_a_native_publication() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("a.rs"), "pub fn first() {}\n").unwrap();
    sync(root.path(), "a.rs");
    // A provider-owned framework node for the same file.
    db(root.path())
        .execute(
            "INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES('component:view','component','View','a.rs::View','a.rs','razor',1,1,0,0,0)",
            [],
        )
        .unwrap();
    fs::write(root.path().join("a.rs"), "pub fn second() {}\n").unwrap();
    let result = sync_file(root.path(), "a.rs", &OperationContext::default()).unwrap();
    assert_eq!(result["committed"], true, "{result}");
    assert_eq!(result["providerNodesPreserved"], 1);
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE id='component:view'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE name='second'"
        ),
        1
    );
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM nodes WHERE name='first'"),
        0
    );
}

#[test]
fn provider_edges_around_framework_nodes_are_republished() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("a.rs"), "pub fn first() {}\n").unwrap();
    fs::write(root.path().join("b.rs"), "pub fn other() {}\n").unwrap();
    sync(root.path(), "a.rs");
    sync(root.path(), "b.rs");
    // A provider framework node in this file with an edge to another file.
    db(root.path())
        .execute_batch(
            "INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES('component:view','component','View','a.rs::View','a.rs','razor',1,1,0,0,0);
             INSERT INTO edges(source,target,kind,metadata) SELECT 'component:view',id,'calls','{\"synthesizedBy\":\"framework\"}' FROM nodes WHERE name='other';",
        )
        .unwrap();
    fs::write(root.path().join("a.rs"), "pub fn second() {}\n").unwrap();
    let result = sync_file(root.path(), "a.rs", &OperationContext::default()).unwrap();
    assert_eq!(result["committed"], true, "{result}");
    assert_eq!(result["providerNodesPreserved"], 1);
    assert_eq!(result["providerEdgesRetained"], 1);
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.source='component:view' AND t.name='other'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE name='second'"
        ),
        1
    );
}
