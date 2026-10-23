#[cfg(windows)]
#[test]
fn local_only_importer_retries_after_global_candidate_ceiling_is_removed() {
    use super::*;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let context = OperationContext::default();
    initialize(root,&context).unwrap();
    let peer = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    for index in 0..70 {
        peer.execute("INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,visibility,updated_at) VALUES(?1,'function','helper','helper',?2,'javascript',1,1,0,1,'public',0)",rusqlite::params![format!("noise-{index}"),format!("noise/{index}.js")]).unwrap();
    }
    std::fs::write(root.join("caller.js"),"import { unused } from './dep'; function helper() {} function caller() { helper(); }\n").unwrap();
    sync_file(root,"caller.js",&context).unwrap();
    let pending: i64 = peer.query_row("SELECT count(*) FROM unresolved_refs WHERE file_path='caller.js' AND reference_name='helper'",[],|row|row.get(0)).unwrap();
    assert_eq!(pending,1);
    peer.execute("DELETE FROM nodes WHERE file_path LIKE 'noise/%'",[]).unwrap();
    std::fs::write(root.join("unrelated.js"),"export function unrelated() {}\n").unwrap();
    sync_file(root,"unrelated.js",&context).unwrap();
    let pending: i64 = peer.query_row("SELECT count(*) FROM unresolved_refs WHERE file_path='caller.js' AND reference_name='helper'",[],|row|row.get(0)).unwrap();
    assert_eq!(pending,0);
}

#[cfg(windows)]
#[test]
fn retry_coverage_observes_peer_commits_and_is_discarded_on_rollback() {
    use super::*;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let context = OperationContext::default();
    initialize(root, &context).unwrap();
    std::fs::write(root.join("target.js"), "export function available() {}\n").unwrap();
    std::fs::write(root.join("caller.js"), "function caller() { missing(); }\n").unwrap();
    let mut writer = None;
    sync_file_config_reuse(root, "target.js", None, &context, || {}, true, &mut writer).unwrap();
    sync_file_config_reuse(root, "caller.js", None, &context, || {}, true, &mut writer).unwrap();
    sync_file_config_reuse(root, "target.js", None, &context, || {}, true, &mut writer).unwrap();
    assert!(
        writer
            .as_ref()
            .unwrap()
            .retry_memo
            .json()
            .contains("available")
    );
    // A peer adds a previously unattempted pending reference without changing
    // declarations. data_version must invalidate our name-coverage shortcut.
    let peer = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    peer.execute("INSERT INTO unresolved_refs(from_node_id,reference_name,reference_kind,line,col,file_path,language) SELECT id,'available','calls',1,1,'caller.js','javascript' FROM nodes WHERE name='caller' AND kind='function'", []).unwrap();
    let result =
        sync_file_config_reuse(root, "target.js", None, &context, || {}, true, &mut writer)
            .unwrap();
    assert_eq!(result["referencesResolved"], 1);
    let pending: i64 = peer
        .query_row(
            "SELECT count(*) FROM unresolved_refs WHERE reference_name='available'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
    let result = sync_file_config_reuse(
        root,
        "target.js",
        None,
        &context,
        || {
            std::fs::write(root.join("target.js"), "export function changed() {}\n").unwrap();
        },
        true,
        &mut writer,
    );
    assert_eq!(result.unwrap_err().0, "STALE");
    assert_eq!(writer.as_ref().unwrap().retry_memo.json(), "[]");
}

#[cfg(windows)]
#[test]
fn script_visibility_changes_and_target_deletion_retry_only_eligible_references() {
    use super::*;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let context = OperationContext::default();
    initialize(root, &context).unwrap();
    std::fs::write(root.join("caller.js"), "function caller() { target(); }\n").unwrap();
    sync_file(root, "caller.js", &context).unwrap();
    std::fs::write(root.join("target.js"), "function target() {}\n").unwrap();
    let private = sync_file(root, "target.js", &context).unwrap();
    assert_eq!(private["referencesAttempted"], 0);
    std::fs::write(root.join("target.js"), "export function target() {}\n").unwrap();
    sync_file(root, "target.js", &context).unwrap();
    fn caller_edges(root: &Path) -> i64 {
        Connection::open(root.join(".codegraph/codegraph.db")).unwrap().query_row("SELECT count(*) FROM edges e JOIN nodes n ON n.id=e.source WHERE n.file_path='caller.js' AND e.kind='calls'",[],|row|row.get(0)).unwrap()
    }
    assert_eq!(caller_edges(root), 1);
    std::fs::remove_file(root.join("target.js")).unwrap();
    sync_file(root, "target.js", &context).unwrap();
    assert_eq!(caller_edges(root), 0);
    std::fs::write(root.join("target.js"), "export function target() {}\n").unwrap();
    sync_file(root, "target.js", &context).unwrap();
    assert_eq!(caller_edges(root), 1);
}

#[cfg(windows)]
#[test]
fn standalone_plan_keeps_the_original_sqlite_error() {
    use super::*;
    let dir = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(dir.path(), &context).unwrap();
    let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch("ALTER TABLE files RENAME COLUMN content_hash TO broken_hash")
        .unwrap();
    drop(db);
    let failure = index_sync::plan_workspace(dir.path(), &context).unwrap_err();
    let diagnostic = take_sync_diagnostic(failure.0, failure.1).unwrap();
    assert_eq!(diagnostic["stage"], "plan_workspace");
    assert!(
        diagnostic["sqlite"]["originalMessage"]
            .as_str()
            .unwrap()
            .contains("content_hash")
    );
}

#[cfg(windows)]
#[test]
fn workspace_reports_errors_after_the_success_report_limit() {
    use super::*;
    let dir = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(dir.path(), &context).unwrap();
    for index in 0..34 {
        std::fs::write(
            dir.path().join(format!("a{index:02}.rs")),
            format!("pub fn f{index}() {{}}\n"),
        )
        .unwrap();
    }
    for name in ["z1.rs", "z2.rs"] {
        std::fs::write(dir.path().join(name), "pub fn broken( {\n").unwrap();
    }
    let report = sync_workspace_all(dir.path(), &context).unwrap();
    assert_eq!(report["files"].as_array().unwrap().len(), 32);
    assert_eq!(report["changedFilesCommitted"], 34);
    assert_eq!(report["failedFiles"], 2);
    let failures = report["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 2);
    assert_eq!(failures[1]["path"], "z2.rs");
    assert!(
        failures[1]["diagnostic"]["extraction"]["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty())
    );
    assert_eq!(report["selectedScopeApplied"], false);
    let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 34);
}

#[cfg(windows)]
#[test]
fn semantic_exclusion_keeps_symbols_and_skips_import_binding() {
    use super::*;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let context = OperationContext::default();
    initialize(root, &context).unwrap();
    std::fs::create_dir(root.join(".agents")).unwrap();
    std::fs::create_dir_all(root.join("Scripts/vendor")).unwrap();
    std::fs::write(root.join("lib.js"), "export function run() {}\n").unwrap();
    sync_file(root, "lib.js", &context).unwrap();
    // Duplicate var declarations are valid legacy JS; unrelated redeclarations
    // must not prevent import binding in the native semantic resolver.
    let source = "import { run } from '../../lib.js'; var x = 1; var x = 2; export function plugin() { run(); }\n";
    std::fs::write(root.join("Scripts/vendor/plugin.js"), source).unwrap();
    let before = sync_file(root, "Scripts/vendor/plugin.js", &context).unwrap();
    assert_eq!(before["changed"], true);
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    let bound_before_exclusion: i64 = db
        .query_row(
            "SELECT count(*) FROM edges WHERE source IN (SELECT id FROM nodes WHERE file_path='Scripts/vendor/plugin.js') AND target IN (SELECT id FROM nodes WHERE file_path='lib.js')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(bound_before_exclusion > 0);
    drop(db);
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
fn semantic_import_collision_warns_and_keeps_syncing_other_files() {
    use super::*;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let context = OperationContext::default();
    initialize(root, &context).unwrap();
    std::fs::write(root.join("lib.js"), "export function run() {}\n").unwrap();
    sync_file(root, "lib.js", &context).unwrap();
    // This is syntactically extractable JS, but the local declaration collides
    // with the imported binding, so semantic import resolution must be partial.
    let source = "import { run } from './lib.js'; const run = () => {}; export function plugin() { run(); }\n";
    std::fs::write(root.join("plugin.js"), source).unwrap();
    let report = sync_file(root, "plugin.js", &context).unwrap();
    assert_eq!(report["graphStatus"], "partial");
    assert_eq!(report["warningFiles"], 1);
    assert_eq!(report["warnings"][0]["code"], "SYNC_SEMANTIC");
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    let plugin: i64 = db
        .query_row(
            "SELECT count(*) FROM nodes WHERE file_path='plugin.js' AND name='plugin'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(plugin, 1);
    let imported: i64 = db
        .query_row(
            "SELECT count(*) FROM edges WHERE source IN (SELECT id FROM nodes WHERE file_path='plugin.js') AND target IN (SELECT id FROM nodes WHERE file_path='lib.js')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(imported, 0);
    std::fs::write(root.join("other.js"), "export function other() {}\n").unwrap();
    assert_eq!(sync_file(root, "other.js", &context).unwrap()["changed"], true);
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
