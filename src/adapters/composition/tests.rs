use super::dependencies;
use crate::adapters::NativeApplicationOptions;
use harness_application::ports::workspace::SourceEditing;
use harness_domain::{contracts::WriteArgs, execution::OperationContext};
use std::sync::{Arc, Mutex};

#[test]
fn default_composition_disables_optional_persistent_adapters() {
    let root = tempfile::tempdir().expect("temporary project root");

    let (workspace, _sessions) = dependencies(root.path(), NativeApplicationOptions::default())
        .expect("compose the native application");

    assert!(workspace.log_store.is_none());
    assert!(workspace.tasks.is_none());
    assert!(workspace.index_watch.is_none());
    assert_eq!(
        workspace.source_queries.project.root,
        workspace.source_editing.project.root
    );
    assert_eq!(
        workspace.source_editing.project.root,
        workspace.session_service.project.root
    );
    assert!(Arc::ptr_eq(
        &workspace.source_queries.external_scopes,
        &workspace.external_scopes,
    ));
    assert!(Arc::ptr_eq(
        &workspace.source_queries.edit_lock,
        &workspace.source_editing.edit_lock,
    ));
}

#[test]
fn composition_injects_requested_log_store_and_watch_status() {
    let root = tempfile::tempdir().expect("temporary project root");
    let watch_status = Arc::new(Mutex::new(serde_json::json!({ "enabled": true })));

    let (workspace, _sessions) = dependencies(
        root.path(),
        NativeApplicationOptions {
            persist_logs: true,
            index_watch: Some(Arc::clone(&watch_status)),
            ..NativeApplicationOptions::default()
        },
    )
    .expect("compose the native application with optional adapters");

    assert!(workspace.log_store.is_some());
    assert!(Arc::ptr_eq(
        workspace
            .index_watch
            .as_ref()
            .expect("watch status injected"),
        &watch_status
    ));
}

#[test]
fn edit_and_synchronized_query_services_exclude_each_other() {
    let root = tempfile::tempdir().expect("temporary project root");
    let (workspace, _sessions) = dependencies(root.path(), NativeApplicationOptions::default())
        .expect("compose the native application");

    let edit_guard = workspace
        .source_editing
        .edit_lock
        .try_lock()
        .expect("edit service acquires the shared mutation lock");
    assert!(workspace.source_queries.edit_lock.try_lock().is_err());
    drop(edit_guard);

    let target = root.path().join("preserved.txt");
    std::fs::write(&target, "original").expect("seed existing workspace file");
    let query_guard = workspace
        .source_queries
        .edit_lock
        .lock()
        .expect("query sync owns the workspace mutation lock");
    let error = workspace
        .source_editing
        .write(
            &WriteArgs {
                path: "preserved.txt".to_owned(),
                content: "replacement".to_owned(),
                bom: None,
                create_parents: false,
                allow_unvalidated_text: true,
                expected_hash: None,
                overwrite: true,
                session: None,
            },
            &OperationContext::default(),
        )
        .expect_err("an edit is rejected while synchronized query owns the lock");
    assert_eq!(error.0, "EDIT_BUSY");
    drop(query_guard);
    assert_eq!(
        std::fs::read_to_string(target).expect("read original file"),
        "original"
    );

    let query_guard = workspace
        .source_queries
        .edit_lock
        .try_lock()
        .expect("query service acquires the shared mutation lock");
    assert!(workspace.source_editing.edit_lock.try_lock().is_err());
    drop(query_guard);
}
