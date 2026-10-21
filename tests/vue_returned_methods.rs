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
fn returned_shorthand_and_const_arrow_methods_resolve_for_alias_and_receiver_calls() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("composables.ts"),
        "export function useClock() { function start() {} const reset = () => {}; return { start, reset }; }\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    fs::write(
        root.path().join("App.vue"),
        "<script setup lang=\"ts\">import { useClock } from './composables'; const { start: launch } = useClock(); const clock = useClock(); export function run() { launch(); clock.reset(); }</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");

    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.file_path='composables.ts' AND t.name IN ('start','reset') AND json_extract(e.metadata,'$.resolvedBy')='vue-composable-return-method-subset-v1'"
        ),
        2
    );
}

#[test]
fn each_unsupported_return_or_receiver_shape_stays_unresolved() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("composables.ts"),
        "export function useClock() { const reset = () => {}; return { reset }; }\n",
    )
    .unwrap();
    fs::write(
        root.path().join("homonym.ts"),
        "export function reset() {}\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    sync(root.path(), "homonym.ts");
    let cases = [
        (
            "export function useClock() { const reset = () => {}; if (ready) return { reset }; return { reset }; }\n",
            "const clock = useClock(); export function run0() { clock.reset(); }",
        ),
        (
            "export function useClock() { const reset = () => {}; return { ...{ reset } }; }\n",
            "const clock = useClock(); export function run1() { clock.reset(); }",
        ),
        (
            "export async function useClock() { const reset = () => {}; return { reset }; }\n",
            "const clock = useClock(); export function run2() { clock.reset(); }",
        ),
        (
            "export function useClock() { return { reset }; const reset = () => {}; }\n",
            "const clock = useClock(); export function run5() { clock.reset(); }",
        ),
        (
            "export function useClock() { function reset() {} reset = replace; return { reset }; }\n",
            "const clock = useClock(); export function run6() { clock.reset(); }",
        ),
    ];
    for (index, (target, caller)) in cases.into_iter().enumerate() {
        fs::write(root.path().join("composables.ts"), target).unwrap();
        sync(root.path(), "composables.ts");
        fs::write(
            root.path().join("App.vue"),
            format!("<script setup lang=\"ts\">import {{ useClock }} from './composables'; {caller}</script>"),
        )
        .unwrap();
        sync(root.path(), "App.vue");
        assert_eq!(
            count(
                root.path(),
                "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.name='reset' AND t.file_path='composables.ts' AND json_extract(e.metadata,'$.resolvedBy')='vue-composable-return-method-subset-v1'"
            ),
            0,
            "unsupported target case {index}"
        );
        assert_eq!(
            count(
                root.path(),
                "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.file_path='homonym.ts'"
            ),
            0,
            "no same-name fallback for target case {index}"
        );
    }

    for (index, usage) in [
        "const clock = useClock(); export function run3() { clock.reset(); pass(clock); }",
        "const clock = useClock(); export function run4() { clock.reset = replace; clock.reset(); }",
        "export const clock = useClock(); export function run7() { clock.reset(); }",
    ]
    .into_iter()
    .enumerate()
    {
        fs::write(
            root.path().join("composables.ts"),
            "export function useClock() { const reset = () => {}; return { reset }; }\n",
        )
        .unwrap();
        sync(root.path(), "composables.ts");
        fs::write(
            root.path().join("App.vue"),
            format!("<script setup lang=\"ts\">import {{ useClock }} from './composables'; {usage}</script>"),
        )
        .unwrap();
        sync(root.path(), "App.vue");
        assert_eq!(
            count(
                root.path(),
                "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.name='reset' AND t.file_path='composables.ts' AND json_extract(e.metadata,'$.resolvedBy')='vue-composable-return-method-subset-v1'"
            ),
            0,
            "unsupported caller case {index}"
        );
        assert_eq!(
            count(
                root.path(),
                "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.file_path='homonym.ts'"
            ),
            0,
            "no same-name fallback for caller case {index}"
        );
    }
}

#[test]
fn type_only_composable_imports_do_not_create_returned_method_edges() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("composables.ts"),
        "export function useClock() { const reset = () => {}; return { reset }; }\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    for import in [
        "import type { useClock } from './composables';",
        "import { type useClock } from './composables';",
    ] {
        fs::write(
            root.path().join("App.vue"),
            format!("<script setup lang=\"ts\">{import} const clock = useClock(); export function run() {{ clock.reset(); }}</script>"),
        )
        .unwrap();
        sync(root.path(), "App.vue");
        assert_eq!(
            count(
                root.path(),
                "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.file_path='composables.ts' AND t.name='reset' AND json_extract(e.metadata,'$.resolvedBy')='vue-composable-return-method-subset-v1'"
            ),
            0,
            "{import}"
        );
    }
}

#[test]
fn const_state_and_async_returned_methods_resolve() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("composables.ts"),
        "export function useCounter() { const count = ref(0); const load = async () => {}; return { count, load }; }\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    fs::write(
        root.path().join("App.vue"),
        "<script setup lang=\"ts\">import { useCounter } from './composables'; const counter = useCounter(); export function run() { counter.load(); }</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.file_path='composables.ts' AND t.name='load' AND json_extract(e.metadata,'$.resolvedBy')='vue-composable-return-method-subset-v1'"
        ),
        1
    );
}

#[test]
fn direct_eval_in_caller_or_composable_disables_method_proof() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("composables.ts"),
        "export function useClock() { function reset() { eval('useClock = other'); } return { reset }; }\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    fs::write(
        root.path().join("App.vue"),
        "<script setup lang=\"ts\">import { useClock } from './composables'; const clock = useClock(); export function run() { clock.reset(); }</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='vue-composable-return-method-subset-v1'"
        ),
        0,
        "direct eval in returned method body can change its lexical proof"
    );

    fs::write(
        root.path().join("composables.ts"),
        "export function useClock() { function reset() {} return { reset }; }\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    fs::write(
        root.path().join("App.vue"),
        "<script setup lang=\"ts\">import { useClock } from './composables'; const clock = useClock(); eval('clock.reset = other'); export function run() { clock.reset(); }</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='vue-composable-return-method-subset-v1'"
        ),
        0,
        "direct eval in Vue script can change receiver state"
    );
    for callee in ["(eval)", "(eval as any)"] {
        fs::write(
            root.path().join("App.vue"),
            format!("<script setup lang=\"ts\">import {{ useClock }} from './composables'; const clock = useClock(); {callee}('clock.reset = other'); export function run() {{ clock.reset(); }}</script>"),
        ).unwrap();
        sync(root.path(), "App.vue");
        assert_eq!(
            count(
                root.path(),
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='vue-composable-return-method-subset-v1'"
            ),
            0,
            "wrapped direct eval: {callee}"
        );
    }
}

#[test]
fn added_then_removed_return_method_target_retries_and_invalidates_edges() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("App.vue"),
        "<script setup lang=\"ts\">import { useClock } from './composables'; const clock = useClock(); export function run() { clock.reset(); }</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='vue-composable-return-method-subset-v1'"
        ),
        0
    );

    fs::write(
        root.path().join("composables.ts"),
        "export const useClock = () => { const reset = () => {}; return { reset }; };\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.file_path='composables.ts' AND t.name='reset' AND json_extract(e.metadata,'$.resolvedBy')='vue-composable-return-method-subset-v1'"
        ),
        1
    );

    fs::remove_file(root.path().join("composables.ts")).unwrap();
    sync(root.path(), "composables.ts");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='vue-composable-return-method-subset-v1'"
        ),
        0
    );
}

#[test]
fn same_named_helpers_from_distinct_composables_keep_exact_declaration_anchors() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("composables.ts"),
        "export function useA() { function refresh() {} return { refresh }; }\nexport function useB() { function refresh() {} return { refresh }; }\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    fs::write(
        root.path().join("App.vue"),
        "<script setup lang=\"ts\">import { useA, useB } from './composables'; const a = useA(); const b = useB(); export function run() { a.refresh(); b.refresh(); }</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");

    assert_eq!(
        count(
            root.path(),
            "SELECT count(DISTINCT e.target) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.file_path='composables.ts' AND t.name='refresh' AND json_extract(e.metadata,'$.resolvedBy')='vue-composable-return-method-subset-v1'"
        ),
        2
    );
}

#[test]
fn barrel_alias_unicode_properties_and_utf16_positions_resolve_exactly() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("composables.ts"),
        "export function useClock() { /* ðŸ😀 */ function rafraîchir() {} return { chargé: rafraîchir }; }\n",
    )
    .unwrap();
    fs::write(
        root.path().join("barrel.ts"),
        "export { useClock as useChrono } from './composables';\n",
    )
    .unwrap();
    sync(root.path(), "composables.ts");
    sync(root.path(), "barrel.ts");
    fs::write(
        root.path().join("App.vue"),
        "<!-- ðŸ😀 -->\r\n<script setup lang=\"ts\">\r\nimport { useChrono } from './barrel';\r\nconst { chargé: démarrer } = useChrono();\r\nexport function run() { const label = 'ðŸ😀'; démarrer(); }\r\n</script>",
    )
    .unwrap();
    sync(root.path(), "App.vue");

    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.file_path='App.vue' AND t.file_path='composables.ts' AND t.name='rafraîchir' AND json_extract(e.metadata,'$.resolvedBy')='vue-composable-return-method-subset-v1' AND json_extract(e.metadata,'$.refName')='démarrer'"
        ),
        1
    );
}
