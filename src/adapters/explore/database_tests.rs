use super::*;
use rusqlite::Connection;
use super::super::INDEX_BUSY;

#[test]
fn sqlite_busy_and_locked_remain_busy_failures_after_the_query_budget() {
    for (code, cause) in [
        (rusqlite::ffi::SQLITE_BUSY, "database_busy"),
        (rusqlite::ffi::SQLITE_LOCKED, "database_locked"),
    ] {
        let diagnostic = RefCell::new(json!({}));
        let context = OperationContext::default();
        let error = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            Some("private SQLite error detail".into()),
        );
        let failure = failure(
            error,
            &diagnostic,
            &context,
            Instant::now() - Duration::from_secs(6),
        );

        assert_eq!(failure, INDEX_BUSY);
        assert_eq!(diagnostic.borrow()["cause"], cause);
        assert!(!failure.1.contains("private SQLite error detail"));
    }
}

#[test]
fn cancellation_and_deadline_take_priority_over_sqlite_busy() {
    let error = || {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            Some("private SQLite error detail".into()),
        )
    };

    let cancelled = OperationContext::default();
    cancelled.cancel();
    let expected = cancelled.check().unwrap_err();
    let cancelled_failure = failure(
        error(),
        &RefCell::new(json!({})),
        &cancelled,
        Instant::now() - Duration::from_secs(6),
    );
    assert_eq!(cancelled_failure.0, expected.0);

    let timed_out = OperationContext::with_timeout(Duration::ZERO);
    let expected = timed_out.check().unwrap_err();
    let timeout_failure = failure(
        error(),
        &RefCell::new(json!({})),
        &timed_out,
        Instant::now() - Duration::from_secs(6),
    );
    assert_eq!(timeout_failure.0, expected.0);
}

#[cfg(windows)]
#[test]
fn read_open_waits_for_an_exclusive_sqlite_transaction_to_release() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("codegraph.db");
    let blocker = Connection::open(&path).unwrap();
    blocker
        .execute_batch(
            "PRAGMA journal_mode=DELETE;
             CREATE TABLE schema_versions(version INTEGER);
             INSERT INTO schema_versions(version) VALUES(11);
             CREATE TABLE nodes(id INTEGER, qualified_name TEXT);
             CREATE TABLE edges(id INTEGER);
             BEGIN EXCLUSIVE;",
        )
        .unwrap();

    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        blocker.execute_batch("ROLLBACK;").unwrap();
    });
    let diagnostic = RefCell::new(json!({}));
    let context = OperationContext::default();
    let started = Instant::now();
    let opened = open(&path, &context, &diagnostic, started);
    release.join().unwrap();

    let opened = opened.unwrap();
    assert_eq!(opened.version, 11);
    assert!(started.elapsed() >= Duration::from_millis(100));
}
