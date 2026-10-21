#[cfg(windows)]
#[test]
fn semantic_exclusion_keeps_symbols_and_prevents_deep_binding_failure() {
    use super::*;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let context = OperationContext::default();
    initialize(root, &context).unwrap();
    std::fs::create_dir(root.join(".agents")).unwrap();
    std::fs::create_dir_all(root.join("Scripts/vendor")).unwrap();
    std::fs::write(root.join("lib.js"), "export function run() {}\n").unwrap();
    sync_file(root, "lib.js", &context).unwrap();
    // Duplicate var declarations are valid legacy JS, but outside this resolver's
    // conservative semantic subset. Basic provider extraction still succeeds.
    let source = "import { run } from '../../lib.js'; var x = 1; var x = 2; export function plugin() { run(); }\n";
    std::fs::write(root.join("Scripts/vendor/plugin.js"), source).unwrap();
    let before = sync_file(root, "Scripts/vendor/plugin.js", &context).unwrap_err();
    assert_eq!(before.0, "SYNC_SEMANTIC");
    std::fs::write(
        root.join(".agents/harness.config.json"),
        r#"{"indexing":{"semanticExclude":["Scripts/vendor/**"]}}"#,
    )
    .unwrap();
    let report = sync_file(root, "Scripts/vendor/plugin.js", &context).unwrap();
    assert_eq!(report["changed"], true);
    // A second pass must also succeed rather than enter periodic retries.
    sync_file(root, "Scripts/vendor/plugin.js", &context).unwrap();
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    let symbols: i64 = db.query_row("SELECT count(*) FROM nodes WHERE file_path='Scripts/vendor/plugin.js' AND name='plugin'", [], |r| r.get(0)).unwrap();
    assert_eq!(symbols, 1);
    let imported: i64 = db.query_row("SELECT count(*) FROM edges WHERE source IN (SELECT id FROM nodes WHERE file_path='Scripts/vendor/plugin.js') AND target IN (SELECT id FROM nodes WHERE file_path='lib.js')", [], |r| r.get(0)).unwrap();
    assert_eq!(imported, 0);
}
#[cfg(windows)]
#[test]
fn reused_writer_preserves_full_durability_and_rejects_late_links() {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    super::initialize(root.path(), &context).unwrap();
    std::fs::write(root.path().join("a.rs"), "pub fn first() {}\n").unwrap();
    std::fs::write(root.path().join("b.rs"), "pub fn second() { first(); }\n").unwrap();
    let mut writer = None;
    let first = super::sync_file_config_reuse(
        root.path(),
        "a.rs",
        None,
        &context,
        || {},
        false,
        &mut writer,
    )
    .unwrap();
    let second = super::sync_file_config_reuse(
        root.path(),
        "b.rs",
        None,
        &context,
        || {},
        false,
        &mut writer,
    )
    .unwrap();
    assert_eq!(first["writerConnectionReused"], false);
    assert_eq!(second["writerConnectionReused"], true);
    let writer = writer.unwrap();
    assert_eq!(
        writer
            .db
            .query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        writer
            .db
            .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(writer.db.is_autocommit());
    // Some Windows filesystems/permissions do not allow this link operation.
    if std::fs::hard_link(
        root.path().join(".codegraph/codegraph.db"),
        root.path().join("alias.db"),
    )
    .is_ok()
    {
        assert_eq!(writer.recheck(&context).unwrap_err().0, "PATH");
    }
}

#[cfg(windows)]
#[test]
fn tsconfig_change_before_commit_rolls_back_nodes_and_configuration_stamp() {
    use super::*;
    for changed in ["tsconfig.json", "base.json"] {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        std::fs::write(dir.path().join("a.ts"), "export function planned() {}\n").unwrap();
        std::fs::write(
            dir.path().join("tsconfig.json"),
            "{\"compilerOptions\":{\"baseUrl\":\".\"}}",
        )
        .unwrap();
        if changed == "base.json" {
            std::fs::write(
                dir.path().join("tsconfig.json"),
                "{\"extends\":\"./base.json\"}",
            )
            .unwrap();
            std::fs::write(dir.path().join("base.json"), "{}").unwrap();
        }
        let result = sync_file_config_with(
            dir.path(),
            "a.ts",
            Some("tsconfig.json"),
            &OperationContext::default(),
            || {
                std::fs::write(
                    dir.path().join(changed),
                    "{\"compilerOptions\":{\"baseUrl\":\"src\"}}",
                )
                .unwrap();
            },
        );
        assert_eq!(result.unwrap_err().0, "STALE");
        let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM nodes WHERE name='planned'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
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
}
use super::*;
fn fixture(root: &Path) {
    std::fs::create_dir_all(root.join(".codegraph")).unwrap();
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(include_str!("../../../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute_batch(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0); PRAGMA journal_mode=WAL;",
    )
    .unwrap();
    std::fs::write(root.join("a.rs"), "pub fn old() {}\n").unwrap();
    sync_file(root, "a.rs", &OperationContext::default()).unwrap();
}
#[test]
fn source_change_after_sql_writes_rolls_back_index() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(dir.path().join("a.rs"), "pub fn planned() {}\n").unwrap();
    let result = sync_file_with(dir.path(), "a.rs", &OperationContext::default(), || {
        std::fs::write(dir.path().join("a.rs"), "pub fn concurrent() {}\n").unwrap();
    });
    assert_eq!(result.unwrap_err().0, "STALE");
    let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM nodes WHERE name='old'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM nodes WHERE name='planned'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn cancellation_after_sql_writes_does_not_interrupt_rollback() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(dir.path().join("a.rs"), "pub fn cancelled() {}\n").unwrap();
    let context = OperationContext::default();
    let result = sync_file_with(dir.path(), "a.rs", &context, || context.cancel());
    assert_eq!(result.unwrap_err().0, "CANCELLED");
    let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM nodes WHERE name='old'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn star_nonwinning_branch_changed_before_commit_rolls_back_target_update() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    for (path, source) in [
        ("lib.ts", "export function api(){}"),
        ("other.ts", "export function unrelated(){}"),
        (
            "barrel.ts",
            "export * from './lib'; export * from './other';",
        ),
        (
            "app.ts",
            "import {api} from './barrel'; export function caller(){api();}",
        ),
    ] {
        std::fs::write(dir.path().join(path), source).unwrap();
        sync_file(dir.path(), path, &OperationContext::default()).unwrap();
    }
    let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
    let before: String = db
        .query_row(
            "SELECT content_hash FROM files WHERE path='lib.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(db);
    std::fs::write(dir.path().join("lib.ts"), "\nexport function api(){}").unwrap();
    let result = sync_file_with(dir.path(), "lib.ts", &OperationContext::default(), || {
        std::fs::write(dir.path().join("other.ts"), "export function api(){}").unwrap();
    });
    assert_eq!(result.unwrap_err().0, "STALE");
    let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT content_hash FROM files WHERE path='lib.ts'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        before
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM edges WHERE kind='calls'", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        1
    );
}
#[test]
fn barrel_changed_after_resolution_rolls_back_target_update() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    for (path, source) in [
        ("lib.ts", "export function run(){}"),
        ("barrel.ts", "export {run as api} from './lib';"),
        (
            "app.ts",
            "import {api} from './barrel'; export function caller(){api();}",
        ),
    ] {
        std::fs::write(dir.path().join(path), source).unwrap();
        sync_file(dir.path(), path, &OperationContext::default()).unwrap();
    }
    let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
    let before: String = db
        .query_row(
            "SELECT content_hash FROM files WHERE path='lib.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(db);
    std::fs::write(dir.path().join("lib.ts"), "\nexport function run(){}").unwrap();
    let result = sync_file_with(dir.path(), "lib.ts", &OperationContext::default(), || {
        std::fs::write(
            dir.path().join("barrel.ts"),
            "export {missing as api} from './lib';",
        )
        .unwrap();
    });
    assert_eq!(result.unwrap_err().0, "STALE");
    let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT content_hash FROM files WHERE path='lib.ts'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        before
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM edges WHERE kind='calls'", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        1
    );
}
#[test]
fn import_source_changed_after_resolution_rolls_back_target_update() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(dir.path().join("lib.ts"), "export function run() {}").unwrap();
    sync_file(dir.path(), "lib.ts", &OperationContext::default()).unwrap();
    std::fs::write(
        dir.path().join("app.ts"),
        "import {run as execute} from './lib'; export function caller() { execute(); }",
    )
    .unwrap();
    sync_file(dir.path(), "app.ts", &OperationContext::default()).unwrap();
    let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
    let before: String = db
        .query_row(
            "SELECT content_hash FROM files WHERE path='lib.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(db);
    std::fs::write(dir.path().join("lib.ts"), "\nexport function run() {}").unwrap();
    let result = sync_file_with(dir.path(), "lib.ts", &OperationContext::default(), || {
        std::fs::write(dir.path().join("app.ts"), "import {run as execute} from './lib'; export function caller(execute: () => void) { execute(); }").unwrap();
    });
    assert_eq!(result.unwrap_err().0, "STALE");
    let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT content_hash FROM files WHERE path='lib.ts'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        before
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM edges WHERE kind='calls'", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        1
    );
}
