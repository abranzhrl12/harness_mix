use super::*;
use std::path::Path;

#[test]
fn reports_non_sqlite_version_conversion_error_with_check_name() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE schema_versions(version TEXT);
         INSERT INTO schema_versions VALUES('not-an-integer');
         CREATE TABLE nodes(id TEXT);
         CREATE TABLE edges(id TEXT);
         CREATE TABLE files(path TEXT);
         CREATE TABLE unresolved_refs(id TEXT);
         CREATE TABLE project_metadata(key TEXT);",
    )
    .unwrap();
    sync_diagnostic::begin("src/test.js", Path::new(".codegraph/codegraph.db"));
    sync_diagnostic::stage("validate_schema");

    let failure = validate_schema(&db).unwrap_err();
    assert_eq!(failure.0, "SYNC_INDEX");
    let diagnostic = sync_diagnostic::take(failure.0, failure.1).unwrap();
    assert_eq!(diagnostic["schemaCheck"], "provider_schema_version_query");
    assert_eq!(diagnostic["sqlite"]["errorType"], "InvalidColumnType");
    assert_eq!(
        diagnostic["schemaValidation"]["check"],
        "provider_schema_version_query"
    );
}
