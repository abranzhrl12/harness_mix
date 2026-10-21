use super::{INDEX, INDEX_BUSY, index_failure_for_sqlite_code};

#[test]
fn busy_and_locked_errors_have_a_specific_diagnostic() {
    assert_eq!(
        index_failure_for_sqlite_code(rusqlite::ErrorCode::DatabaseBusy),
        INDEX_BUSY
    );
    assert_eq!(
        index_failure_for_sqlite_code(rusqlite::ErrorCode::DatabaseLocked),
        INDEX_BUSY
    );
}

#[test]
fn other_sqlite_errors_keep_the_generic_index_diagnostic() {
    assert_eq!(
        index_failure_for_sqlite_code(rusqlite::ErrorCode::ConstraintViolation),
        INDEX
    );
}
