use super::{
    IndexWatch,
    events::relevant,
    pending::{MAX_PENDING, MAX_WAIT, Pending},
    report::{retry_delay, sync_report},
};
use crate::adapters::index_store;
use harness_domain::execution::OperationContext;
use notify::{Event, EventKind};
use serde_json::json;
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !predicate() {
        assert!(Instant::now() < deadline, "watcher did not converge");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn count(root: &Path, query: &str) -> i64 {
    rusqlite::Connection::open_with_flags(
        root.join(".codegraph/codegraph.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()
    .and_then(|db| db.query_row(query, [], |r| r.get(0)).ok())
    .unwrap_or(-1)
}

#[test]
fn watcher_busy_means_windows_sharing_or_lock_violation() {
    assert!(super::lock_contended(&std::io::Error::from_raw_os_error(
        32
    )));
    assert!(super::lock_contended(&std::io::Error::from_raw_os_error(
        33
    )));
    assert!(!super::lock_contended(&std::io::Error::from_raw_os_error(
        5
    )));
}

#[test]
fn enabling_watcher_initializes_and_indexes_a_new_project() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("initial.rs"), "pub fn initial() {}\n").unwrap();
    let watch = IndexWatch::start(root.path()).unwrap();
    wait(|| watch.status.lock().unwrap()["state"] == "idle");
    assert_eq!(count(root.path(), "SELECT count(*) FROM nodes WHERE name='initial'"), 1);
}

#[test]
fn supervisor_waits_for_an_existing_incomplete_namespace_without_overwriting_it() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join(".codegraph");
    std::fs::create_dir(&directory).unwrap();
    let marker = directory.join("incomplete-marker");
    std::fs::write(&marker, b"preserve").unwrap();
    std::fs::write(root.path().join("initial.rs"), "pub fn initial() {}\n").unwrap();
    let watch = super::supervisor::SupervisedWatch::start(root.path()).unwrap();
    wait(|| watch.status.lock().unwrap()["state"] == "retrying");
    assert_eq!(std::fs::read(&marker).unwrap(), b"preserve");
    std::fs::remove_file(marker).unwrap();
    std::fs::remove_dir(directory).unwrap();
    wait(|| count(root.path(), "SELECT count(*) FROM nodes WHERE name='initial'") == 1);
}

#[test]
fn malformed_razor_script_does_not_replace_previous_indexed_source() {
    let root = tempfile::tempdir().unwrap();
    index_store::initialize(root.path(), &OperationContext::default()).unwrap();
    let source = root.path().join("page.cshtml");
    std::fs::write(&source, "<script>fetch('/before');</script>").unwrap();
    let watch = IndexWatch::start(root.path()).unwrap();
    wait(|| watch.status.lock().unwrap()["state"] == "idle");
    let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    let previous: String = db.query_row("SELECT content_hash FROM files WHERE path='page.cshtml'", [], |row| row.get(0)).unwrap();
    std::fs::write(&source, "<script>function broken( {</script>\n<script>fetch('/valid');</script>").unwrap();
    wait(|| watch.status.lock().unwrap()["state"] == "retrying");
    let current: String = db.query_row("SELECT content_hash FROM files WHERE path='page.cshtml'", [], |row| row.get(0)).unwrap();
    assert_eq!(current, previous);
    assert_eq!(watch.status.lock().unwrap()["lastError"], "SYNC_PARTIAL");
}

#[test]
fn partial_javascript_is_not_published_as_a_complete_new_file() {
    let root = tempfile::tempdir().unwrap();
    index_store::initialize(root.path(), &OperationContext::default()).unwrap();
    std::fs::write(root.path().join("legacy.js"), "function broken( {\nfetch('/valid');\n").unwrap();
    let watch = IndexWatch::start(root.path()).unwrap();
    wait(|| watch.status.lock().unwrap()["state"] == "retrying");
    assert_eq!(count(root.path(), "SELECT count(*) FROM files WHERE path='legacy.js'"), 0);
    assert_eq!(watch.status.lock().unwrap()["lastError"], "SYNC_PARTIAL");
    assert_eq!(watch.status.lock().unwrap()["globalFreshnessCertified"], false);
}

#[test]
fn partial_javascript_recovery_preserves_previous_complete_graph() {
    let root = tempfile::tempdir().unwrap();
    index_store::initialize(root.path(), &OperationContext::default()).unwrap();
    let source = root.path().join("legacy.js");
    std::fs::write(&source, "function previous() {}\n").unwrap();
    let watch = IndexWatch::start(root.path()).unwrap();
    wait(|| watch.status.lock().unwrap()["state"] == "idle");
    assert_eq!(count(root.path(), "SELECT count(*) FROM nodes WHERE name='previous'"), 1);
    std::fs::write(&source, "function broken( {\nfetch('/valid');\n").unwrap();
    wait(|| watch.status.lock().unwrap()["state"] == "retrying");
    assert_eq!(count(root.path(), "SELECT count(*) FROM nodes WHERE name='previous'"), 1);
    assert_eq!(watch.status.lock().unwrap()["globalFreshnessCertified"], false);
}

#[test]
fn native_watcher_updates_only_changed_files_handles_rename_delete_and_releases_lease() {
    let root = tempfile::tempdir().unwrap();
    index_store::initialize(root.path(), &OperationContext::default()).unwrap();
    std::fs::write(root.path().join("a.rs"), "pub fn before() {}\n").unwrap();
    std::fs::write(root.path().join("b.rs"), "pub fn untouched() {}\n").unwrap();
    std::fs::write(root.path().join(".gitignore"), "ignored.rs\n").unwrap();
    let watch = IndexWatch::start(root.path()).unwrap();
    let (code, message) = match IndexWatch::start(root.path()) {
        Ok(_) => panic!("one automatic writer per project"),
        Err(failure) => failure,
    };
    assert_eq!(code, "SYNC_WATCH_BUSY");
    assert!(message.contains("harness_explore"));
    assert!(message.contains("not verified"));
    wait(|| watch.status.lock().unwrap()["state"] == "idle");
    assert_eq!(count(root.path(), "SELECT count(*) FROM files"), 2);
    let untouched = count(
        root.path(),
        "SELECT indexed_at FROM files WHERE path='b.rs'",
    );
    for _ in 0..8 {
        std::fs::write(root.path().join("a.rs"), "pub fn after() {}\n").unwrap();
    }
    wait(|| count(root.path(), "SELECT count(*) FROM nodes WHERE name='after'") == 1);
    assert_eq!(
        count(
            root.path(),
            "SELECT indexed_at FROM files WHERE path='b.rs'"
        ),
        untouched
    );
    assert_eq!(watch.status.lock().unwrap()["reconciliations"], 1);
    std::fs::write(root.path().join("a.rs"), "fn incomplete( {").unwrap();
    wait(|| watch.status.lock().unwrap()["state"] == "retrying");
    {
        let status = watch.status.lock().unwrap();
        assert_eq!(status["lastResult"]["failure"]["path"], "a.rs");
        assert!(status["lastResult"]["failure"]["message"].is_string());
        assert_eq!(status["retryScheduled"], true);
    }
    // Keep the last confirmed graph while the editor holds incomplete syntax.
    assert_eq!(
        count(root.path(), "SELECT count(*) FROM nodes WHERE name='after'"),
        1
    );
    std::fs::write(root.path().join("a.rs"), "pub fn recovered() {}\n").unwrap();
    wait(|| {
        count(
            root.path(),
            "SELECT count(*) FROM nodes WHERE name='recovered'",
        ) == 1
    });
    assert_eq!(watch.status.lock().unwrap()["reconciliations"], 1);
    std::fs::rename(root.path().join("a.rs"), root.path().join("renamed.rs")).unwrap();
    wait(|| {
        count(
            root.path(),
            "SELECT count(*) FROM files WHERE path='renamed.rs'",
        ) == 1
            && count(root.path(), "SELECT count(*) FROM files WHERE path='a.rs'") == 0
    });
    std::fs::remove_file(root.path().join("renamed.rs")).unwrap();
    wait(|| {
        count(
            root.path(),
            "SELECT count(*) FROM files WHERE path='renamed.rs'",
        ) == 0
    });
    let batches = watch.status.lock().unwrap()["batches"].as_u64().unwrap();
    std::fs::write(root.path().join("ignored.rs"), "pub fn ignored() {}\n").unwrap();
    wait(|| watch.status.lock().unwrap()["batches"].as_u64().unwrap() > batches);
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM files WHERE path='ignored.rs'"
        ),
        0
    );
    drop(watch);
    let restarted = IndexWatch::start(root.path()).unwrap();
    wait(|| restarted.status.lock().unwrap()["state"] == "idle");
}

#[test]
fn incomplete_coverage_waits_for_events_and_retains_its_diagnosis() {
    let root = tempfile::tempdir().unwrap();
    index_store::initialize(root.path(), &OperationContext::default()).unwrap();
    std::fs::write(root.path().join("a.rs"), "pub fn before() {}\n").unwrap();
    std::fs::write(root.path().join("large.rs"), vec![b' '; 1024 * 1024 + 1]).unwrap();
    let watch = IndexWatch::start(root.path()).unwrap();
    wait(|| watch.status.lock().unwrap()["state"] == "degraded");
    {
        let status = watch.status.lock().unwrap();
        assert_eq!(status["lastError"], "SYNC_INCOMPLETE");
        assert_eq!(status["lastReconciliation"]["uncheckedFiles"], 1);
        assert_eq!(status["retryScheduled"], false);
        assert!(status["retryDelayMs"].is_null());
    }
    // Longer than the old initial backoff: no repeated whole-workspace scan.
    std::thread::sleep(Duration::from_secs(6));
    assert_eq!(watch.status.lock().unwrap()["reconciliations"], 1);
    std::fs::write(root.path().join("a.rs"), "pub fn after() {}\n").unwrap();
    wait(|| count(root.path(), "SELECT count(*) FROM nodes WHERE name='after'") == 1);
    wait(|| watch.status.lock().unwrap()["batches"].as_u64().unwrap() > 0);
    {
        let status = watch.status.lock().unwrap();
        assert_eq!(status["state"], "degraded");
        assert_eq!(status["lastReconciliation"]["uncheckedFiles"], 1);
        assert_eq!(status["reconciliations"], 1);
    }
    // Removing the coverage obstacle and reconciling clears the diagnosis.
    std::fs::remove_file(root.path().join("large.rs")).unwrap();
    {
        let (mutex, wake) = &*watch.queue;
        mutex.lock().unwrap().reconcile();
        wake.notify_one();
    }
    wait(|| watch.status.lock().unwrap()["state"] == "idle");
    assert_eq!(
        watch.status.lock().unwrap()["lastReconciliation"]["complete"],
        true
    );
}

#[test]
fn retry_policy_is_bounded_and_preserves_failure_details() {
    for code in [
        "SYNC_UNSUPPORTED_RELATION",
        "SYNC_SCHEMA",
        "SYNC_LIMIT",
        "SYNC_CONFIG",
        "SYNC_COMMIT_UNCERTAIN",
    ] {
        let report = sync_report(&Err((code, "Needs a change")), true);
        assert_eq!(report["lastError"], code);
        assert_eq!(report["failure"]["message"], "Needs a change");
        assert!(retry_delay(&report, 1).is_none());
    }
    let report = sync_report(&Err(("STALE", "Source changed")), false);
    for (failures, seconds) in [(1, 5), (2, 10), (3, 20), (4, 40)] {
        assert_eq!(
            retry_delay(&report, failures),
            Some(Duration::from_secs(seconds))
        );
    }
    // A watcher deadline is transient: unlike deterministic inventory limits,
    // it must schedule another reconciliation instead of leaving the index stale.
    let timed_out = sync_report(&Err(("TIMEOUT", "Operation deadline elapsed")), true);
    assert_eq!(timed_out["retryable"], true);
    assert_eq!(retry_delay(&timed_out, 1), Some(Duration::from_secs(5)));
    assert!(retry_delay(&report, 0).is_none());
    assert!(retry_delay(&report, 5).is_none());
    assert!(retry_delay(&report, u32::MAX).is_none());
    let report = sync_report(
        &Ok(json!({
            "selectedScopeApplied":false,
            "failure":{"path":"src/a.rs","code":"STALE","message":"Source changed","commitState":"not_committed"},
            "changedFilesCommitted":2
        })),
        false,
    );
    assert_eq!(report["failure"]["path"], "src/a.rs");
    assert_eq!(report["changedFilesCommitted"], 2);
    assert!(retry_delay(&report, 1).is_some());
}

#[test]
fn coverage_limitations_are_not_transient_failures() {
    let clean = json!({"selectedScopeApplied":true,"discoveryComplete":true,
        "uncheckedFiles":0,"unsupportedChangedFiles":0,"restrictedIndexedFiles":0});
    assert_eq!(sync_report(&Ok(clean.clone()), true)["complete"], true);
    for (key, value) in [
        ("discoveryComplete", json!(false)),
        ("uncheckedFiles", json!(1)),
        ("unsupportedChangedFiles", json!(1)),
    ] {
        let mut incomplete = clean.clone();
        incomplete[key] = value;
        let report = sync_report(&Ok(incomplete), true);
        assert_eq!(report["complete"], false);
        assert_eq!(report["lastError"], "SYNC_INCOMPLETE");
        assert!(retry_delay(&report, 1).is_none());
    }
    assert_eq!(
        sync_report(&Ok(json!({"selectedScopeApplied":true})), false)["complete"],
        true
    );
    let mut excluded = clean;
    excluded["restrictedIndexedFiles"] = json!(3);
    excluded["restrictedIndexedReasons"] = json!({"config_filename":3});
    let report = sync_report(&Ok(excluded), true);
    assert_eq!(report["complete"], true);
    assert_eq!(report["restrictedIndexedReasons"]["config_filename"], 3);
    assert!(report["lastError"].is_null());
    assert!(retry_delay(&report, 1).is_none());
}

#[test]
fn archived_graph_warnings_do_not_schedule_identical_failed_retries() {
    let report = sync_report(
        &Ok(
            json!({"sourceScopeApplied":true,"selectedScopeApplied":false,
        "discoveryComplete":true,"uncheckedFiles":0,"unsupportedChangedFiles":0,
        "warningFiles":1,"warnings":[{"code":"INDEX_DERIVED_RELATIONS_INVALIDATED"}]}),
        ),
        true,
    );
    assert_eq!(report["complete"], false);
    assert_eq!(report["graphStatus"], "partial");
    assert_eq!(report["warningFiles"], 1);
    assert!(report["lastError"].is_null());
    assert!(retry_delay(&report, 1).is_none());
}

#[test]
fn configuration_events_and_rescan_flags_schedule_reconciliation() {
    let root = Path::new("C:/project");
    for name in ["codegraph.json", "tsconfig.json", "src/.gitignore"] {
        let mut pending = Pending::default();
        relevant(
            root,
            Event::new(EventKind::Any).add_path(root.join(name)),
            &mut pending,
        );
        assert!(pending.full);
    }
    let mut pending = Pending::default();
    let mut event = Event::new(EventKind::Other);
    event.attrs.set_flag(notify::event::Flag::Rescan);
    relevant(root, event, &mut pending);
    assert!(pending.full);
}

#[test]
fn bursts_are_bounded_and_generated_events_do_not_schedule_work() {
    let root = Path::new("C:/project");
    let mut pending = Pending::default();
    for path in [
        "target/a.rs",
        ".codegraph/codegraph.db-wal",
        ".agents/log.rs",
        ".agents",
        ".env",
        "README.md",
    ] {
        relevant(
            root,
            Event::new(EventKind::Any).add_path(root.join(path)),
            &mut pending,
        );
    }
    assert!(pending.first.is_none());
    for _ in 0..100 {
        pending.add("src/a.rs".into());
    }
    assert_eq!(pending.paths.len(), 1);
    assert!(pending.ready_at().unwrap() <= pending.first.unwrap() + MAX_WAIT);
    for i in 0..MAX_PENDING + 1 {
        pending.add(format!("src/a{i}.rs"));
    }
    assert!(pending.full);
    assert!(pending.paths.is_empty());
}
