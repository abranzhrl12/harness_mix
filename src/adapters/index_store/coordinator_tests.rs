use super::*;
use std::{fs, sync::mpsc, time::{Duration, Instant}};

#[test]
fn competing_rebuild_waits_out_a_transaction_longer_than_sqlite_busy_timeout() {
    let root = tempfile::tempdir().unwrap();
    initialize(root.path(), &OperationContext::default()).unwrap();
    fs::write(root.path().join("a.js"), "export function indexed() {}").unwrap();
    let path = root.path().to_owned();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first = std::thread::spawn(move || sync_file_with(&path, "a.js",
        &OperationContext::default(), || {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(20)).unwrap();
        }));
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let path = root.path().to_owned();
    let (waiting_tx, waiting_rx) = mpsc::channel();
    let context = OperationContext::default().with_progress(move |p| {
        if p.phase == "waiting_index_writer" { let _ = waiting_tx.send(()); }
    });
    let second = std::thread::spawn(move || rebuild_workspace(&path, &context));
    let waiting = waiting_rx.recv_timeout(Duration::from_secs(2));
    let held = Instant::now();
    std::thread::sleep(Duration::from_millis(5_500));
    release_tx.send(()).unwrap();
    assert_eq!(first.join().unwrap().unwrap()["committed"], true);
    let result = second.join().unwrap().unwrap();
    assert!(waiting.is_ok(), "rebuild bypassed the coordinator");
    assert!(held.elapsed() >= Duration::from_secs(5));
    assert_eq!(result["selectedScopeApplied"], true, "{result}");
    assert_eq!(result["failedFiles"], 0, "{result}");
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    assert_eq!(db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)).unwrap(), "ok");
    assert_eq!(db.query_row("SELECT count(*) FROM files", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
}
