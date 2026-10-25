use super::{DirectoryPinner, PATH, Store};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    fs,
    fs::File,
    path::{Path, PathBuf},
};

struct TestPinner;

impl DirectoryPinner for TestPinner {
    fn pin_directory(
        &self,
        root: &Path,
        path: &Path,
    ) -> Result<Vec<File>, harness_domain::Failure> {
        if !path.starts_with(root) {
            return Err(PATH);
        }
        path.ancestors()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|ancestor| {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::OpenOptionsExt;
                    File::options()
                        .read(true)
                        .share_mode(3)
                        .custom_flags(0x0200_0000 | 0x0020_0000)
                        .open(ancestor)
                        .map_err(|_| PATH)
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    File::options()
                        .read(true)
                        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                        .open(ancestor)
                        .map_err(|_| PATH)
                }
                #[cfg(not(any(windows, unix)))]
                {
                    File::open(ancestor).map_err(|_| PATH)
                }
            })
            .collect()
    }
}

fn store_create(root: &Path) -> Store {
    Store::create(root, &TestPinner).unwrap()
}

fn store_open(root: &Path) -> Result<Option<Store>, harness_domain::Failure> {
    Store::open_existing(root, &TestPinner)
}

struct CountingPinner(AtomicUsize);

impl DirectoryPinner for CountingPinner {
    fn pin_directory(
        &self,
        root: &Path,
        path: &Path,
    ) -> Result<Vec<File>, harness_domain::Failure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        TestPinner.pin_directory(root, path)
    }
}

struct FailingPinner(AtomicUsize);

impl DirectoryPinner for FailingPinner {
    fn pin_directory(
        &self,
        _root: &Path,
        _path: &Path,
    ) -> Result<Vec<File>, harness_domain::Failure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(PATH)
    }
}

#[test]
fn storage_uses_pinner_for_root_and_each_managed_directory() {
    let root = tempfile::tempdir().unwrap();
    let pinner = CountingPinner(AtomicUsize::new(0));
    let store = Store::create(root.path(), &pinner).unwrap();
    assert_eq!(pinner.0.load(Ordering::SeqCst), 4);
    drop(store);
}

#[test]
fn pinner_failure_fails_closed_before_opening_existing_storage() {
    let root = tempfile::tempdir().unwrap();
    let pinner = FailingPinner(AtomicUsize::new(0));
    assert!(matches!(
        Store::open_existing(root.path(), &pinner),
        Err(("TASK_STORE_PATH", _))
    ));
    assert_eq!(pinner.0.load(Ordering::SeqCst), 1);
    assert!(!root.path().join(".agents").exists());
}

#[test]
fn writer_busy_means_os_reported_lock_contention_only() {
    #[cfg(windows)]
    {
        assert!(super::filesystem::lock_contended(
            &std::io::Error::from_raw_os_error(32)
        ));
        assert!(super::filesystem::lock_contended(
            &std::io::Error::from_raw_os_error(33)
        ));
        assert!(!super::filesystem::lock_contended(
            &std::io::Error::from_raw_os_error(5)
        ));
    }
    #[cfg(unix)]
    {
        assert!(super::filesystem::lock_contended(&std::io::Error::from(
            std::io::ErrorKind::WouldBlock,
        )));
        assert!(!super::filesystem::lock_contended(&std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        )));
    }
}

fn crash_snapshot() -> Value {
    json!({"tasks":(0..64).map(|index| json!({"taskId":index,"padding":"x".repeat(100 * 1024)})).collect::<Vec<_>>(),"events":[],"cursor":1})
}
#[test]
fn hot_journal_fixture() {
    let Some(root) = std::env::var_os("HARNESS_TASK_STORE_CRASH_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let store = store_create(&root);
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
            "store::tests::hot_journal_fixture",
            "--nocapture",
        ])
        .env("HARNESS_TASK_STORE_CRASH_ROOT", root.path())
        .status()
        .unwrap();
    assert!(status.success());
    let store = store_open(root.path()).unwrap().unwrap();
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
    assert!(store_open(root.path()).unwrap().is_none());
    assert!(!root.path().join(".agents").exists());
    let store = store_create(root.path());
    assert_eq!(
        fs::read(root.path().join(".agents/harness/jobs/.gitignore")).unwrap(),
        b"*\n"
    );
    store.save(&snapshot(1)).unwrap();
    assert!(matches!(
        store_open(root.path()),
        Err(("TASK_STORE_BUSY", _))
    ));
    drop(store);
    let store = store_open(root.path()).unwrap().unwrap();
    assert_eq!(store.load().unwrap(), snapshot(1));
}
#[test]
fn failed_transaction_preserves_last_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let store = store_create(root.path());
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
    let store = store_create(root.path());
    drop(store);
    let path = root.path().join(".agents/harness/jobs/jobs.db");
    fs::write(&path, b"fixture corrupt db").unwrap();
    assert!(store_open(root.path()).is_err());
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
        let store = store_create(root.path());
        drop(store);
        let path = root.path().join(".agents/harness/jobs").join(name);
        fs::hard_link(&path, root.path().join("linked")).unwrap();
        assert!(store_open(root.path()).is_err());
    }
}
