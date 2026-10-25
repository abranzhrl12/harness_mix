use super::dependencies;
use crate::adapters::NativeApplicationOptions;
use harness_application::ports::workspace::{SourceEditing, SourceQueries};
use harness_domain::{
    contracts::{ExploreArgs, WriteArgs},
    execution::OperationContext,
};
use std::sync::{Arc, Mutex};

#[test]
fn composition_uses_injected_browser_without_initializing_native_connection() {
    struct FakeBrowser;
    impl harness_application::ports::browser::BrowserInspection for FakeBrowser {}
    let root = tempfile::tempdir().expect("temporary project root");
    let browser: Arc<dyn harness_application::ports::browser::BrowserInspection> =
        Arc::new(FakeBrowser);
    let (workspace, _) = dependencies(
        root.path(),
        NativeApplicationOptions {
            browser: Some(browser.clone()),
            ..NativeApplicationOptions::default()
        },
    )
    .expect("compose injected browser");
    assert!(Arc::ptr_eq(&browser, &workspace.browser));
}

#[test]
#[cfg(not(feature = "browser-cdp"))]
fn composition_without_browser_feature_reports_backend_unavailable() {
    let root = tempfile::tempdir().expect("temporary project root");
    let (workspace, _) = dependencies(root.path(), NativeApplicationOptions::default())
        .expect("compose without browser dependency");
    let error = workspace
        .browser
        .browser(
            &harness_domain::browser::BrowserArgs::Connect {
                endpoint: "ws://127.0.0.1:9222/devtools/browser/test".into(),
                session: Some("test".into()),
            },
            &OperationContext::default(),
        )
        .expect_err("browser backend is disabled");
    assert_eq!(error.0, "UNSUPPORTED");
    assert!(error.1.contains("--features browser-cdp"));
}

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
        &workspace.index_maintenance.edit_lock,
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
fn edits_and_index_maintenance_exclude_each_other() {
    let root = tempfile::tempdir().expect("temporary project root");
    let (workspace, _sessions) = dependencies(root.path(), NativeApplicationOptions::default())
        .expect("compose the native application");

    let edit_guard = workspace
        .source_editing
        .edit_lock
        .try_lock()
        .expect("edit service acquires the shared mutation lock");
    assert!(workspace.index_maintenance.edit_lock.try_lock().is_err());
    drop(edit_guard);

    let target = root.path().join("preserved.txt");
    std::fs::write(&target, "original").expect("seed existing workspace file");
    let maintenance_guard = workspace
        .index_maintenance
        .edit_lock
        .lock()
        .expect("index maintenance owns the workspace mutation lock");
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
        .expect_err("an edit is rejected while index maintenance owns the lock");
    assert_eq!(error.0, "EDIT_BUSY");
    drop(maintenance_guard);
    assert_eq!(
        std::fs::read_to_string(target).expect("read original file"),
        "original"
    );

    let maintenance_guard = workspace
        .index_maintenance
        .edit_lock
        .try_lock()
        .expect("index maintenance acquires the shared mutation lock");
    assert!(workspace.source_editing.edit_lock.try_lock().is_err());
    drop(maintenance_guard);
}

#[test]
fn explore_only_uses_maintenance_lock_when_sync_is_explicit() {
    let root = tempfile::tempdir().expect("temporary project root");
    let (workspace, _) = dependencies(root.path(), NativeApplicationOptions::default())
        .expect("compose the native application");
    let lock = Arc::clone(&workspace.index_maintenance.edit_lock);
    let _ = std::thread::spawn(move || {
        let _guard = lock.lock().expect("acquire mutation lock");
        panic!("poison mutation lock for dispatch regression");
    })
    .join();
    let args = |sync_index| ExploreArgs {
        query: "needle".to_owned(),
        view: None,
        file_path: None,
        container: None,
        symbol_id: None,
        sync_index,
        symbol: None,
        max_files: None,
        max_chars: None,
        direction: None,
        max_depth: None,
        limit: None,
        require_matching_files: None,
        session: None,
    };

    let synchronized = workspace
        .explore(&args(Some(true)), &OperationContext::default())
        .expect_err("explicit sync dispatches to index maintenance");
    assert_eq!(synchronized.0, "INTERNAL");

    let read_only = workspace
        .explore(&args(Some(false)), &OperationContext::default())
        .expect("read-only explore returns its bounded index diagnostic");
    assert_eq!(read_only["diagnostics"][0]["code"], "INDEX");
    assert!(!root.path().join(".codegraph").exists());
}
