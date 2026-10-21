use super::dependencies;
use crate::adapters::NativeApplicationOptions;
use std::sync::{Arc, Mutex};

#[test]
fn default_composition_disables_optional_persistent_adapters() {
    let root = tempfile::tempdir().expect("temporary project root");

    let (workspace, _sessions) = dependencies(root.path(), NativeApplicationOptions::default())
        .expect("compose the native application");

    assert!(workspace.log_store.is_none());
    assert!(workspace.tasks.is_none());
    assert!(workspace.index_watch.is_none());
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
