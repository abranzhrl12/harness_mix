use super::Store;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn crash_snapshot() -> Value {
    json!({"tasks":(0..64).map(|index| json!({"taskId":index,"padding":"x".repeat(100 * 1024)})).collect::<Vec<_>>(),"events":[],"cursor":1})
}
#[test]
fn hot_journal_fixture() {
    let Some(root) = std::env::var_os("HARNESS_TASK_STORE_CRASH_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let store = Store::create(&root).unwrap();
    store.save(&crash_snapshot()).unwrap();
    let path = root.join(".agents/harness/jobs/jobs.db");
    let before = harness_domain::hash(&fs::read(&path).unwrap());
    store
        .db
        .execute_batch("PRAGMA cache_size=10; BEGIN IMMEDIATE;")
        .unwrap();
    store
        .db
        .execute(
            "UPDATE tasks SET data=?1",
            [json!({"padding":"y".repeat(100 * 1024)}).to_string()],
        )
        .unwrap();
    assert!(
        fs::metadata(root.join(".agents/harness/jobs/jobs.db-journal"))
            .unwrap()
            .len()
            > 0
    );
    assert_ne!(
        before,
        harness_domain::hash(&fs::read(path).unwrap()),
        "dirty pages must spill before the crash"
    );
    // Deliberately bypass Store/SQLite destructors, leaving an actual hot journal.
    std::process::exit(0);
}
#[test]
fn hot_journal_recovers_after_process_crash() {
    let root = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "adapters::tasks_store::tests::hot_journal_fixture",
            "--nocapture",
        ])
        .env("HARNESS_TASK_STORE_CRASH_ROOT", root.path())
        .status()
        .unwrap();
    assert!(status.success());
    let store = Store::open_existing(root.path()).unwrap().unwrap();
    assert_eq!(store.load().unwrap(), crash_snapshot());
    store.save(&snapshot(2)).unwrap();
    assert_eq!(store.load().unwrap(), snapshot(2));
}
fn snapshot(cursor: u64) -> Value {
    json!({"tasks":[{"id":"fixture","status":"succeeded"}],"events":[{"cursor":cursor,"kind":"state"}],"cursor":cursor})
}
#[test]
fn lazy_storage_reopens_and_excludes_second_writer() {
    let root = tempfile::tempdir().unwrap();
    assert!(Store::open_existing(root.path()).unwrap().is_none());
    assert!(!root.path().join(".agents").exists());
    let store = Store::create(root.path()).unwrap();
    assert_eq!(
        fs::read(root.path().join(".agents/harness/jobs/.gitignore")).unwrap(),
        b"*\n"
    );
    store.save(&snapshot(1)).unwrap();
    assert!(matches!(
        Store::open_existing(root.path()),
        Err(("TASK_STORE_BUSY", _))
    ));
    drop(store);
    let store = Store::open_existing(root.path()).unwrap().unwrap();
    assert_eq!(store.load().unwrap(), snapshot(1));
}
#[test]
fn failed_transaction_preserves_last_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::create(root.path()).unwrap();
    store.save(&snapshot(1)).unwrap();
    store.db.execute_batch("CREATE TEMP TRIGGER reject_state BEFORE UPDATE ON state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(store.save(&snapshot(2)).is_err());
    assert_eq!(store.load().unwrap(), snapshot(1));
    assert!(
        store
            .save(&json!({"tasks":[],"events":[],"cursor":u64::MAX}))
            .is_err()
    );
}
#[test]
fn corrupt_database_is_not_reinitialized() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::create(root.path()).unwrap();
    drop(store);
    let path = root.path().join(".agents/harness/jobs/jobs.db");
    fs::write(&path, b"fixture corrupt db").unwrap();
    assert!(Store::open_existing(root.path()).is_err());
    assert_eq!(fs::read(path).unwrap(), b"fixture corrupt db");
}
#[test]
fn rejects_hard_linked_database_and_lock() {
    for name in [
        "jobs.db",
        "writer.lock",
        "jobs.db-journal",
        "jobs.db-wal",
        "jobs.db-shm",
    ] {
        let root = tempfile::tempdir().unwrap();
        let store = Store::create(root.path()).unwrap();
        drop(store);
        let path = root.path().join(".agents/harness/jobs").join(name);
        fs::hard_link(&path, root.path().join("linked")).unwrap();
        assert!(Store::open_existing(root.path()).is_err());
    }
}
