use super::*;
use crate::adapters::index_store::{initialize, sync_file_config_prepared};
use rusqlite::Connection;

struct FailingInvalidator;

impl<'db, 'scope> ReferenceInvalidatorPort<Transaction<'db>, InvalidationScope<'scope>>
    for FailingInvalidator
{
    type Outcome = InvalidationOutcome;

    fn invalidate(
        &self,
        _: &Transaction<'db>,
        _: &OperationContext,
        _: InvalidationScope<'scope>,
    ) -> Result<Self::Outcome, Failure> {
        Err(("TEST_INVALIDATION", "Injected invalidation failure"))
    }
}

#[test]
fn injected_failure_rolls_back_publication() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let context = OperationContext::without_deadline();
    initialize(root, &context).unwrap();
    std::fs::write(root.join("target.js"), "export function target() {}\n").unwrap();
    let error = sync_file_config_prepared(
        root,
        "target.js",
        None,
        &context,
        || {},
        true,
        &mut None,
        None,
        &FailingInvalidator,
    )
    .unwrap_err();
    assert_eq!(error.0, "TEST_INVALIDATION");
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    for table in ["files", "nodes", "edges", "unresolved_refs"] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "partial writes leaked in {table}");
    }
}

#[test]
fn batch_and_incremental_preserve_cross_file_retries_and_deletion() {
    fn run(invalidator: &ReferenceInvalidator) -> (i64, i64) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let context = OperationContext::without_deadline();
        initialize(root, &context).unwrap();
        let mut writer = None;
        std::fs::write(
            root.join("caller.js"),
            "export function caller() { target(); }\n",
        )
        .unwrap();
        sync_file_config_prepared(
            root,
            "caller.js",
            None,
            &context,
            || {},
            true,
            &mut writer,
            None,
            invalidator,
        )
        .unwrap();
        std::fs::write(root.join("target.js"), "export function target() {}\n").unwrap();
        sync_file_config_prepared(
            root,
            "target.js",
            None,
            &context,
            || {},
            true,
            &mut writer,
            None,
            invalidator,
        )
        .unwrap();
        let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
        let calls = || {
            db.query_row("SELECT count(*) FROM edges WHERE kind='calls'", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
        };
        let resolved = calls();
        std::fs::remove_file(root.join("target.js")).unwrap();
        sync_file_config_prepared(
            root,
            "target.js",
            None,
            &context,
            || {},
            true,
            &mut writer,
            None,
            invalidator,
        )
        .unwrap();
        (resolved, calls())
    }
    assert_eq!(run(&IncrementalSqliteInvalidator), (1, 0));
    assert_eq!(run(&BatchSqliteInvalidator), (1, 0));
}
