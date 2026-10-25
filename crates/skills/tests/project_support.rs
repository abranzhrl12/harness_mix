use harness_skills::project_support::ProjectSupport;
use std::path::Path;

#[test]
fn initialization_support_owns_lock_and_never_replaces_existing_files() {
    let temporary = tempfile::tempdir().unwrap();
    let root = ProjectSupport::checked_root(temporary.path()).unwrap();
    let relative = ".agents/example.json";
    let value = serde_json::json!({"example": true});

    {
        let support = ProjectSupport::lock(&root).unwrap();
        assert!(ProjectSupport::lock(&root).is_err());
        support.write_new_json(relative, &value).unwrap();
        assert!(support.write_new_json(relative, &value).is_err());
        let report = support.sync().unwrap();
        assert_eq!(report.errors, 0);
        assert_eq!(report.conflicts, 0);
        support.set_auto_sync(true).unwrap();
    }

    assert!(ProjectSupport::lock(&root).is_ok());
    let bytes = ProjectSupport::read_file(&root, Path::new(relative))
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        value
    );
    assert!(
        ProjectSupport::preferences(&root)
            .unwrap()
            .unwrap()
            .auto_sync
    );
    assert_eq!(ProjectSupport::preview(&root).unwrap().changes, 0);
}
