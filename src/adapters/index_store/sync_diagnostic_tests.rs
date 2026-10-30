use super::*;

#[test]
fn profile_tracks_distinct_phases_and_finishes_once() {
    begin("sample.rs", Path::new(".codegraph/codegraph.db"));
    CURRENT.with(|slot| slot.borrow_mut().as_mut().unwrap().phase_started = Some(Instant::now()));
    stage("extract_source");
    stage("write_nodes_and_edges");
    let profile = finish_profile().unwrap();
    assert!(profile["open_database"].is_number());
    assert!(profile["extract_source"].is_number());
    assert!(profile["write_nodes_and_edges"].is_number());
    assert!(finish_profile().is_none());
}

#[test]
fn reports_context_for_schema_failures_without_sqlite_exception() {
    begin("src/app.js", Path::new(".codegraph/codegraph.db"));
    stage("validate_schema");
    schema(0, 42, Some(7));
    let diagnostic = take("SYNC_SCHEMA", "unsupported schema").unwrap();
    assert_eq!(diagnostic["failureCode"], "SYNC_SCHEMA");
    assert_eq!(diagnostic["stage"], "validate_schema");
    assert_eq!(diagnostic["file"], "src/app.js");
    assert_eq!(diagnostic["databasePath"], ".codegraph/codegraph.db");
    assert_eq!(diagnostic["schema"]["providerSchemaVersion"], 7);
    assert!(diagnostic["sqlite"].is_null());
}

#[test]
fn retains_non_sqlite_rusqlite_errors_and_validation_check() {
    begin("src/app.js", Path::new(".codegraph/codegraph.db"));
    stage("validate_schema");
    schema_check("provider_schema_version_query");
    capture(&Error::InvalidColumnType(
        0,
        "version".to_owned(),
        rusqlite::types::Type::Null,
    ));
    let diagnostic = take("SYNC_INDEX", "schema validation failed").unwrap();
    assert_eq!(diagnostic["schemaCheck"], "provider_schema_version_query");
    assert_eq!(
        diagnostic["schemaValidation"]["check"],
        "provider_schema_version_query"
    );
    assert_eq!(diagnostic["sqlite"]["errorType"], "InvalidColumnType");
    assert!(
        diagnostic["sqlite"]["originalMessage"]
            .as_str()
            .unwrap()
            .contains("version")
    );
    assert_eq!(diagnostic["sqlite"]["sqliteCode"], Value::Null);
}

#[test]
fn successful_schema_validation_is_not_reported_as_failed() {
    begin("src/app.js", Path::new(".codegraph/codegraph.db"));
    stage("validate_schema");
    schema_check("fts_definition_query");
    schema_validated();

    let diagnostic = take("SYNC_INDEX", "later indexing failure").unwrap();
    assert_eq!(diagnostic["schemaCheck"], "fts_definition_query");
    assert_eq!(diagnostic["schemaValidation"]["check"], "fts_definition_query");
    assert_eq!(diagnostic["schemaValidation"]["outcome"], "validated");
}
