use super::{select_engine, select_from_availability};
use crate::adapters::project::Project;
use harness_domain::sql::SqlEngine;
use std::path::PathBuf;

#[test]
fn explicit_engine_bypasses_availability_configuration() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join(".agents")).unwrap();
    std::fs::write(root.path().join(".agents/harness.config.json"), b"{").unwrap();
    let project = Project {
        root: PathBuf::from(root.path()),
        external: false,
        single_file: None,
    };

    assert_eq!(
        select_engine(&project, Some(SqlEngine::Postgresql)),
        Ok(SqlEngine::Postgresql)
    );
    assert_eq!(
        select_engine(&project, Some(SqlEngine::SqlServer)),
        Ok(SqlEngine::SqlServer)
    );
}

#[test]
fn default_engine_selection_preserves_ambiguity_and_fallback() {
    assert_eq!(
        select_from_availability(true, true).unwrap_err().0,
        "SQL_ENGINE_REQUIRED"
    );
    assert_eq!(
        select_from_availability(true, false),
        Ok(SqlEngine::Postgresql)
    );
    assert_eq!(
        select_from_availability(false, true),
        Ok(SqlEngine::SqlServer)
    );
    assert_eq!(
        select_from_availability(false, false),
        Ok(SqlEngine::SqlServer)
    );
}
