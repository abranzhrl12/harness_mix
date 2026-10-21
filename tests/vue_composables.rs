#![cfg(windows)]
use agent_harness::adapters::index_store::sync_file;
use harness_domain::execution::OperationContext;
use rusqlite::Connection;
use std::{fs, path::Path};

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

fn sync(root: &Path, path: &str) {
    sync_file(root, path, &OperationContext::default()).unwrap();
}

fn count(root: &Path, query: &str) -> i64 {
    Connection::open(root.join(".codegraph/codegraph.db"))
        .unwrap()
        .query_row(query, [], |row| row.get(0))
        .unwrap()
}

#[test]
fn named_composable_alias_resolves_to_its_barrel_export_not_a_homonym() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("composables.ts"),
        "export function useClock() {}\n",
    )
    .unwrap();
    fs::write(
        root.path().join("barrel.ts"),
        "export { useClock as useTimer } from './composables';\n",
    )
    .unwrap();
    fs::write(
        root.path().join("homonym.ts"),
        "export function useTimer() {}\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    sync(root.path(), "barrel.ts");
    sync(root.path(), "homonym.ts");
    fs::write(
        root.path().join("App.vue"),
        "<script setup lang=\"ts\">import { useTimer as useLocalTimer } from './barrel';\nexport function run() { useLocalTimer(); }</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");

    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.file_path='composables.ts' AND t.name='useClock' AND json_extract(e.metadata,'$.refName')='useLocalTimer' AND json_extract(e.metadata,'$.resolvedBy')='vue-named-composable-subset-v1'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.file_path='homonym.ts'"
        ),
        0
    );
}

#[test]
fn const_arrow_composable_is_a_runtime_function_target() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("composables.ts"),
        "export const useClock = () => {};\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    fs::write(
        root.path().join("App.vue"),
        "<script setup lang=\"ts\">import { useClock } from './composables'; export function run() { useClock(); }</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");

    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='composables.ts' AND t.name='useClock' AND json_extract(e.metadata,'$.resolvedBy')='vue-named-composable-subset-v1'"
        ),
        1
    );
}

#[test]
fn unicode_alias_name_matches_the_same_lexical_import_binding() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("composables.ts"),
        "export function useClock() {}\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    fs::write(
        root.path().join("App.vue"),
        "<script setup lang=\"ts\">import { useClock as useCafé } from './composables'; export function run() { useCafé(); }</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");

    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='composables.ts' AND t.name='useClock' AND json_extract(e.metadata,'$.refName')='useCafé'"
        ),
        1
    );
}

#[test]
fn lexical_shadowing_and_utf16_coordinates_resolve_only_the_imported_call() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("composables.ts"),
        "export function useClock() {}\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    let source = "<!-- 😀 -->\r\n<script setup lang=\"ts\">\r\nimport { useClock as useLocalTimer } from './composables';\r\nexport function local(useLocalTimer: () => void) { useLocalTimer(); }\r\nexport function run() { const label = '😀'; useLocalTimer(); }\r\n</script>";
    fs::write(root.path().join("App.vue"), source).unwrap();
    sync(root.path(), "App.vue");

    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.file_path='composables.ts' AND t.name='useClock'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs r JOIN nodes s ON s.id=r.from_node_id WHERE r.file_path='App.vue' AND r.reference_name='useLocalTimer' AND r.reference_kind='calls' AND s.name='local'"
        ),
        1
    );
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    let (line, col): (i64, i64) = db
        .query_row(
            "SELECT e.line,e.col FROM edges e JOIN nodes s ON s.id=e.source WHERE e.kind='calls' AND s.file_path='App.vue'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let expected_line = 5;
    let expected_col = source
        .split('\n')
        .nth((expected_line - 1) as usize)
        .unwrap()
        .trim_end_matches('\r')
        .split("useLocalTimer();")
        .next()
        .unwrap()
        .encode_utf16()
        .count() as i64;
    assert_eq!((line, col), (expected_line, expected_col));
}

#[test]
fn normal_and_setup_scripts_keep_their_import_bindings_separate() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    for (path, name) in [("first.ts", "useFirst"), ("second.ts", "useSecond")] {
        fs::write(
            root.path().join(path),
            format!("export function {name}() {{}}\n"),
        )
        .unwrap();
        sync(root.path(), path);
    }
    fs::write(
        root.path().join("App.vue"),
        "<script lang=\"ts\">import { useFirst as useIt } from './first'; export function normal() { useIt(); }</script>\n<script setup lang=\"ts\">import { useSecond as useIt } from './second'; export function setup() { useIt(); }</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");

    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='first.ts' AND t.name='useFirst'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='second.ts' AND t.name='useSecond'"
        ),
        1
    );
}

#[test]
fn identical_aliases_in_two_scripts_do_not_cross_resolve() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("valid.ts"),
        "export function useClock() {}\n",
    )
    .unwrap();
    sync(root.path(), "valid.ts");
    fs::write(
        root.path().join("App.vue"),
        "<script lang=\"ts\">import { useClock as useIt } from './missing'; export function first() { useIt(); }</script>\n<script setup lang=\"ts\">import { useClock as useIt } from './valid'; export function second() { useIt(); }</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");

    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.file_path='valid.ts' AND t.name='useClock'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs r JOIN nodes s ON s.id=r.from_node_id WHERE r.file_path='App.vue' AND r.reference_name='useIt' AND r.reference_kind='calls' AND s.name='first'"
        ),
        1
    );
}

#[test]
fn late_composable_target_retries_the_pending_vue_call() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("App.vue"),
        "<script setup lang=\"ts\">import { useClock } from './composables'; export function run() { useClock(); }</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM edges WHERE kind='calls'"),
        0
    );

    fs::write(
        root.path().join("composables.ts"),
        "export function useClock() {}\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='composables.ts' AND t.name='useClock'"
        ),
        1
    );
}

#[test]
fn unsupported_import_forms_remain_pending_without_global_fallback() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("library.ts"),
        "export function useClock() {}\n",
    )
    .unwrap();
    fs::write(
        root.path().join("homonym.ts"),
        "export function useClock() {}\n",
    )
    .unwrap();
    sync(root.path(), "library.ts");
    sync(root.path(), "homonym.ts");

    for (index, import) in [
        "import useClock from './library'; useClock();",
        "import * as hooks from './library'; hooks();",
        "import type { useClock } from './library'; useClock();",
    ]
    .into_iter()
    .enumerate()
    {
        fs::write(
            root.path().join("App.vue"),
            format!(
                "<script setup lang=\"ts\">{import} export function run{index}() {{}}</script>"
            ),
        )
        .unwrap();
        sync(root.path(), "App.vue");
        assert_eq!(
            count(
                root.path(),
                "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.name='useClock'"
            ),
            0,
            "{import}"
        );
    }
}

#[test]
fn semantic_exclude_disables_vue_import_call_resolution() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir_all(root.path().join(".agents")).unwrap();
    fs::write(
        root.path().join(".agents/harness.config.json"),
        r#"{"indexing":{"semanticExclude":["App.vue"]}}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("composables.ts"),
        "export function useClock() {}\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    fs::write(
        root.path().join("App.vue"),
        "<script setup lang=\"ts\">import { useClock } from './composables'; export function run() { useClock(); }</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");

    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.file_path='composables.ts'"
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs r JOIN nodes s ON s.id=r.from_node_id WHERE r.file_path='App.vue' AND r.reference_name='useClock' AND r.reference_kind='calls' AND s.name='run'"
        ),
        1
    );
}
