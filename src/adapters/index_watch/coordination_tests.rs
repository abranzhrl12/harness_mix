use super::IndexWatch;
use crate::adapters::index_store;
use harness_domain::execution::OperationContext;
use std::{fs::{self, File}, path::Path, time::{Duration, Instant}};

fn gate(root: &Path) -> File {
    let file = File::options().read(true).write(true).create(true).truncate(false)
        .open(root.join(".codegraph/index-write.lock")).unwrap();
    file.lock().unwrap();
    file
}

fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if predicate() {
            return;
        }
        assert!(Instant::now() < deadline, "watcher did not converge");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn names(root: &Path) -> Vec<String> {
    let db = rusqlite::Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    let mut query = db.prepare("SELECT name FROM nodes WHERE kind='function' ORDER BY name").unwrap();
    query.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

#[test]
#[ignore]
fn watcher_owner_process_helper() {
    use std::io::{self, Write};
    let root = std::env::var_os("HARNESS_WATCH_OWNER_ROOT").unwrap();
    let watch = super::supervisor::SupervisedWatch::start(Path::new(&root)).unwrap();
    wait(|| watch.status.lock().unwrap()["role"] == "owner");
    println!("WATCH_OWNER_READY");
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
}

#[test]
fn standby_recovers_after_owner_process_is_terminated() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::os::windows::process::CommandExt;
    let root = tempfile::tempdir().unwrap();
    index_store::initialize(root.path(), &OperationContext::default()).unwrap();
    fs::write(root.path().join("a.js"), "export function before() {}").unwrap();
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
    }
    let mut child = ChildGuard(Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--nocapture", "watcher_owner_process_helper"])
        .env("HARNESS_WATCH_OWNER_ROOT", root.path())
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .creation_flags(0x08000000).spawn().unwrap());
    let stdout = child.0.stdout.take().unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.contains("WATCH_OWNER_READY") { let _ = ready_tx.send(()); }
        }
    });
    ready_rx.recv_timeout(Duration::from_secs(15)).expect("owner did not start");
    let standby = super::supervisor::SupervisedWatch::start(root.path()).unwrap();
    wait(|| standby.status.lock().unwrap()["role"] == "standby");
    wait(|| standby.status.lock().unwrap()["observedOwner"]["pid"] == child.0.id());
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    reader.join().unwrap();
    fs::write(root.path().join("a.js"), "export function after() {}").unwrap();
    wait(|| standby.status.lock().unwrap()["role"] == "owner");
    wait(|| names(root.path()) == vec!["after"]);
}

#[test]
fn bad_telemetry_does_not_stop_indexing_or_takeover() {
    let root = tempfile::tempdir().unwrap();
    index_store::initialize(root.path(), &OperationContext::default()).unwrap();
    let status_path = root.path().join(".codegraph/autosync.status");
    fs::write(&status_path, b"foreign").unwrap();
    fs::write(root.path().join("a.js"), "export function before() {}").unwrap();
    let owner = super::supervisor::SupervisedWatch::start(root.path()).unwrap();
    let standby = super::supervisor::SupervisedWatch::start(root.path()).unwrap();
    wait(|| owner.status.lock().unwrap()["telemetryPublished"] == false);
    wait(|| names(root.path()) == vec!["before"]);
    drop(owner);
    fs::write(root.path().join("a.js"), "export function after() {}").unwrap();
    wait(|| standby.status.lock().unwrap()["role"] == "owner");
    wait(|| names(root.path()) == vec!["after"]);
    assert_eq!(fs::read(status_path).unwrap(), b"foreign");
}

#[test]
fn multiple_standbys_elect_one_replacement() {
    let root = tempfile::tempdir().unwrap();
    index_store::initialize(root.path(), &OperationContext::default()).unwrap();
    let owner = super::supervisor::SupervisedWatch::start(root.path()).unwrap();
    let first = super::supervisor::SupervisedWatch::start(root.path()).unwrap();
    let second = super::supervisor::SupervisedWatch::start(root.path()).unwrap();
    wait(|| first.status.lock().unwrap()["role"] == "standby" && second.status.lock().unwrap()["role"] == "standby");
    drop(owner);
    wait(|| first.status.lock().unwrap()["role"] == "owner" || second.status.lock().unwrap()["role"] == "owner");
    std::thread::sleep(Duration::from_millis(2200));
    let roles = [first.status.lock().unwrap()["role"].clone(), second.status.lock().unwrap()["role"].clone()];
    assert_eq!(roles.iter().filter(|r| **r == "owner").count(), 1, "{roles:?}");
}

#[test]
fn standby_takes_over_after_owner_stops_and_indexes_new_files() {
    let root = tempfile::tempdir().unwrap();
    index_store::initialize(root.path(), &OperationContext::default()).unwrap();
    fs::write(root.path().join("a.js"), "export function before() {}").unwrap();
    let owner = super::supervisor::SupervisedWatch::start(root.path()).unwrap();
    let standby = super::supervisor::SupervisedWatch::start(root.path()).unwrap();
    wait(|| owner.status.lock().unwrap()["role"] == "owner");
    wait(|| standby.status.lock().unwrap()["role"] == "standby");
    drop(owner);
    fs::write(root.path().join("a.js"), "export function after() {}").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while standby.status.lock().unwrap()["role"] != "owner" {
        assert!(Instant::now() < deadline, "standby did not take over; status={}", standby.status.lock().unwrap());
        std::thread::sleep(Duration::from_millis(25));
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while names(root.path()) != vec!["after"] {
        assert!(
            Instant::now() < deadline,
            "owner did not index new files; names={:?}, status={}",
            names(root.path()),
            standby.status.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn watcher_retains_changes_received_while_cli_owns_the_gate() {
    let root = tempfile::tempdir().unwrap();
    index_store::initialize(root.path(), &OperationContext::default()).unwrap();
    fs::write(root.path().join("a.js"), "export function before() {}").unwrap();
    let owner = gate(root.path());
    let watch = IndexWatch::start(root.path()).unwrap();
    {
        let deadline = Instant::now() + Duration::from_secs(15);
        while watch.status.lock().unwrap()["state"] != "syncing" {
            assert!(Instant::now() < deadline, "watcher never started syncing; status={}", watch.status.lock().unwrap());
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    assert!(names(root.path()).is_empty());
    fs::write(root.path().join("a.js"), "export function after() {}").unwrap();
    fs::write(root.path().join("b.js"), "export function added() {}").unwrap();
    drop(owner);
    let deadline = Instant::now() + Duration::from_secs(15);
    while names(root.path()) != vec!["added", "after"] {
        assert!(Instant::now() < deadline, "names={:?}, status={}", names(root.path()), watch.status.lock().unwrap());
        std::thread::sleep(Duration::from_millis(25));
    }
    // Committed rows may become visible before a retry scheduled during the
    // coordinator handoff has completed. Wait for the required terminal state.
    while watch.status.lock().unwrap()["state"] != "idle" {
        assert!(Instant::now() < deadline, "watcher did not become idle; names={:?}, status={}", names(root.path()), watch.status.lock().unwrap());
        std::thread::sleep(Duration::from_millis(25));
    }
    let final_state = watch.status.lock().unwrap()["state"].clone();
    assert_eq!(final_state, "idle", "status={}", watch.status.lock().unwrap());
    assert_eq!(watch.status.lock().unwrap()["lastError"], serde_json::Value::Null);
}

#[test]
fn dropping_watcher_cancels_a_coordinator_wait_and_releases_handles() {
    let root = tempfile::tempdir().unwrap();
    index_store::initialize(root.path(), &OperationContext::default()).unwrap();
    fs::write(root.path().join("a.js"), "export function indexed() {}").unwrap();
    let owner = gate(root.path());
    let watch = IndexWatch::start(root.path()).unwrap();
    wait(|| watch.status.lock().unwrap()["state"] == "syncing");
    let started = Instant::now();
    drop(watch);
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    assert!(names(root.path()).is_empty());
    drop(owner);
    let report = index_store::sync_workspace_all(root.path(), &OperationContext::default()).unwrap();
    assert_eq!(report["selectedScopeApplied"], true, "{report}");
    assert_eq!(names(root.path()), vec!["indexed"]);
}
