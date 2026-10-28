#![cfg(windows)]
// Regression: every public publication entry must honor the same OS-backed gate.
use agent_harness::adapters::index_store::{
    initialize, rebuild_workspace, sync_changed_paths, sync_file, sync_file_forced,
    sync_file_with_tsconfig, sync_workspace, sync_workspace_all, take_sync_diagnostic,
};
use harness_domain::{Failure, execution::OperationContext};
use rusqlite::Connection;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    initialize(root.path(), &OperationContext::default()).unwrap();
    fs::write(root.path().join("a.js"), "export function indexed() {}").unwrap();
    root
}

fn gate(root: &Path) -> File {
    let file = File::options().read(true).write(true).create(true).truncate(false)
        .open(root.join(".codegraph/index-write.lock")).unwrap();
    file.lock().unwrap();
    file
}

fn assert_integrity(root: &Path, expected_files: i64) {
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    assert_eq!(db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)).unwrap(), "ok");
    assert_eq!(db.query_row("SELECT count(*) FROM files", [], |r| r.get::<_, i64>(0)).unwrap(), expected_files);
    assert!(!db.prepare("PRAGMA foreign_key_check").unwrap().exists([]).unwrap());
}

fn blocked(call: impl FnOnce(&Path, &OperationContext) -> Result<Value, Failure>) {
    let root = fixture();
    let _owner = gate(root.path());
    let context = OperationContext::with_timeout(Duration::from_millis(400));
    let started = Instant::now();
    let result = call(root.path(), &context);
    assert_eq!(result.as_ref().err().map(|e| e.0), Some("TIMEOUT"), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    let diagnostic = take_sync_diagnostic("TIMEOUT", "deadline").unwrap();
    assert_eq!(diagnostic["stage"], "wait_writer_coordinator");
    assert_integrity(root.path(), 0);
}

#[test]
fn full_sync_waits_before_planning() { blocked(sync_workspace_all); }
#[test]
fn bounded_sync_waits_before_planning() { blocked(sync_workspace); }
#[test]
fn rebuild_waits_before_planning() { blocked(rebuild_workspace); }
#[test]
fn single_file_waits_before_reading_sources() {
    blocked(|root, context| sync_file(root, "a.js", context));
}
#[test]
fn forced_file_waits_before_reading_sources() {
    blocked(|root, context| sync_file_forced(root, "a.js", context));
}
#[test]
fn explicit_config_file_waits_before_reading_sources() {
    blocked(|root, context| sync_file_with_tsconfig(root, "a.js", "tsconfig.json", context));
}
#[test]
fn incremental_batch_waits_before_opening_writer() {
    blocked(|root, context| sync_changed_paths(root, &BTreeSet::from(["a.js".into()]), context));
}
#[test]
fn sql_file_uses_the_same_gate() {
    blocked(|root, context| sync_file(root, "schema.sql", context));
}

#[test]
fn cancellation_interrupts_wait_without_publishing() {
    let root = fixture();
    let _owner = gate(root.path());
    let context = OperationContext::without_deadline();
    let cancel = context.clone();
    let (tx, rx) = mpsc::channel();
    let context = context.with_progress(move |p| {
        if p.phase == "waiting_index_writer" { let _ = tx.send(()); }
    });
    let path = root.path().to_owned();
    let worker = std::thread::spawn(move || sync_workspace_all(&path, &context));
    let observed = rx.recv_timeout(Duration::from_secs(2));
    cancel.cancel();
    let result = worker.join().unwrap();
    assert!(observed.is_ok(), "operation did not report coordinator contention");
    assert_eq!(result.unwrap_err().0, "CANCELLED");
    assert_integrity(root.path(), 0);
}

#[test]
fn waiter_continues_after_release_and_readers_remain_available() {
    let root = fixture();
    let owner = gate(root.path());
    let (tx, rx) = mpsc::channel();
    let context = OperationContext::default().with_progress(move |p| {
        if p.phase == "waiting_index_writer" { let _ = tx.send(()); }
    });
    let path = root.path().to_owned();
    let worker = std::thread::spawn(move || sync_workspace_all(&path, &context));
    let observed = rx.recv_timeout(Duration::from_secs(2));
    assert_integrity(root.path(), 0);
    drop(owner);
    let result = worker.join().unwrap().unwrap();
    assert!(observed.is_ok(), "operation bypassed the gate");
    assert_eq!(result["selectedScopeApplied"], true, "{result}");
    assert_integrity(root.path(), 1);
}

#[test]
fn a_different_workspace_does_not_wait_for_the_owner() {
    let first = fixture();
    let _owner = gate(first.path());
    let second = fixture();
    let result = sync_workspace_all(second.path(), &OperationContext::default()).unwrap();
    assert_eq!(result["selectedScopeApplied"], true, "{result}");
    assert_integrity(second.path(), 1);
}

#[test]
fn worktree_inventory_fallback_reuses_the_outer_gate() {
    let root = fixture();
    fs::write(root.path().join(".git"), "gitdir: unused-for-native-inventory").unwrap();
    let result = sync_changed_paths(root.path(), &BTreeSet::from(["a.js".into()]),
        &OperationContext::with_timeout(Duration::from_secs(2))).unwrap();
    assert_eq!(result["selectedScopeApplied"], true, "{result}");
    assert_integrity(root.path(), 1);
}

#[test]
fn publication_holds_the_gate_through_final_progress_and_releases_it_on_return() {
    let root = fixture();
    let path = root.path().to_owned();
    let (tx, rx) = mpsc::channel();
    let context = OperationContext::default().with_progress(move |p| {
        if p.phase == "finalizing" {
            let file = File::options().read(true).write(true)
                .open(path.join(".codegraph/index-write.lock")).unwrap();
            assert!(matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
            tx.send(()).unwrap();
        }
    });
    let result = sync_workspace_all(root.path(), &context).unwrap();
    assert_eq!(result["selectedScopeApplied"], true);
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let _next_owner = gate(root.path());
}

#[test]
fn single_file_failure_releases_the_gate_for_next_operation() {
    let root = fixture();
    assert!(sync_file_with_tsconfig(root.path(), "a.js", "absent.json",
        &OperationContext::default()).is_err());
    let result = sync_file(root.path(), "a.js", &OperationContext::default()).unwrap();
    assert_eq!(result["committed"], true, "{result}");
    assert_integrity(root.path(), 1);
}

#[test]
fn nonempty_gate_is_preserved_and_publication_is_refused() {
    let root = fixture();
    let path = root.path().join(".codegraph/index-write.lock");
    fs::write(&path, "user content").unwrap();
    assert_eq!(sync_workspace_all(root.path(), &OperationContext::default()).unwrap_err().0, "PATH");
    assert_eq!(fs::read_to_string(path).unwrap(), "user content");
    assert_integrity(root.path(), 0);
}

#[test]
fn hardlinked_gate_is_refused() {
    let root = fixture();
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("sentinel");
    fs::write(&sentinel, b"").unwrap();
    fs::hard_link(&sentinel, root.path().join(".codegraph/index-write.lock")).unwrap();
    assert_eq!(sync_workspace_all(root.path(), &OperationContext::default()).unwrap_err().0, "PATH");
    assert_eq!(fs::read(sentinel).unwrap(), b"");
    assert_integrity(root.path(), 0);
}

#[test]
fn missing_index_does_not_create_coordination_state() {
    let root = tempfile::tempdir().unwrap();
    assert!(sync_workspace_all(root.path(), &OperationContext::default()).is_err());
    assert!(!root.path().join(".codegraph").exists());
}

#[test]
fn case_aliases_wait_on_the_same_gate() {
    let root = fixture();
    let _owner = gate(root.path());
    let alias = root.path().to_string_lossy().to_uppercase();
    let result = sync_workspace_all(Path::new(&alias),
        &OperationContext::with_timeout(Duration::from_millis(400)));
    assert_eq!(result.unwrap_err().0, "TIMEOUT");
    assert_integrity(root.path(), 0);
}

#[test]
fn gate_cannot_be_replaced_while_a_publisher_waits() {
    let root = fixture();
    let owner = gate(root.path());
    let path = root.path().to_owned();
    let (tx, rx) = mpsc::channel();
    let context = OperationContext::default().with_progress(move |p| {
        if p.phase == "waiting_index_writer" { let _ = tx.send(()); }
    });
    let worker = std::thread::spawn(move || sync_workspace_all(&path, &context));
    let ready = rx.recv_timeout(Duration::from_secs(5));
    let deletion = fs::remove_file(root.path().join(".codegraph/index-write.lock"));
    drop(owner);
    let result = worker.join().unwrap().unwrap();
    assert!(ready.is_ok());
    assert!(deletion.is_err(), "coordination identity could be replaced");
    assert_eq!(result["selectedScopeApplied"], true);
}

#[test]
fn wait_is_bounded_even_without_an_operation_deadline() {
    let root = fixture();
    let _owner = gate(root.path());
    let started = Instant::now();
    let result = sync_workspace_all(root.path(), &OperationContext::without_deadline());
    assert_eq!(result.unwrap_err().0, "SYNC_INDEX_BUSY");
    assert!(started.elapsed() < Duration::from_secs(40), "{:?}", started.elapsed());
    assert_integrity(root.path(), 0);
}

// Child process uses a real OS lock and stdin as a release barrier.
#[test]
#[ignore]
fn cooperative_owner_process() {
    let root = std::env::var_os("HARNESS_TEST_INDEX_OWNER_ROOT").expect("test child only");
    let _owner = gate(Path::new(&root));
    println!("INDEX_OWNER_READY");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    let _ = std::io::stdin().read(&mut byte);
}

struct Owner(Child);
impl Owner {
    fn start(root: &Path) -> Self {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "cooperative_owner_process", "--ignored", "--nocapture"])
            .env("HARNESS_TEST_INDEX_OWNER_ROOT", root)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
        let mut owner = Self(command.spawn().unwrap());
        let stdout = owner.0.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if line.unwrap().contains("INDEX_OWNER_READY") { let _ = tx.send(()); }
            }
        });
        rx.recv_timeout(Duration::from_secs(10)).expect("child lock acquisition");
        owner
    }
}
impl Drop for Owner {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}

#[test]
fn real_cli_rebuild_waits_then_publishes_after_another_process_releases() {
    use std::os::windows::process::CommandExt;
    let root = fixture();
    let mut owner = Owner::start(root.path());
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-harness"));
    command.arg("--root").arg(root.path()).args(["--index", "rebuild"])
        .creation_flags(0x08000000).stdin(Stdio::null())
        .stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut cli = Owner(command.spawn().unwrap());
    let stderr = cli.0.stderr.take().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut output = String::new();
        for line in BufReader::new(stderr).lines() {
            let line = line.unwrap();
            if line.contains("waiting_index_writer") { let _ = ready_tx.send(()); }
            output.push_str(&line);
            output.push('\n');
        }
        output
    });
    ready_rx.recv_timeout(Duration::from_secs(10)).expect("CLI waiting on coordinator");
    assert!(cli.0.try_wait().unwrap().is_none());
    assert_integrity(root.path(), 0);
    owner.0.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(owner.0.wait().unwrap().success());
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = cli.0.try_wait().unwrap() { break status; }
        assert!(Instant::now() < deadline, "CLI did not finish after owner released");
        std::thread::sleep(Duration::from_millis(25));
    };
    let stderr = reader.join().unwrap();
    let mut stdout = String::new();
    cli.0.stdout.take().unwrap().read_to_string(&mut stdout).unwrap();
    assert!(status.success(), "{stderr}\n{stdout}");
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["selectedScopeApplied"], true, "{report}");
    assert_integrity(root.path(), 1);
}

#[test]
fn another_process_releases_the_gate_normally() {
    let root = fixture();
    let mut owner = Owner::start(root.path());
    let context = OperationContext::with_timeout(Duration::from_millis(400));
    assert_eq!(sync_workspace_all(root.path(), &context).unwrap_err().0, "TIMEOUT");
    owner.0.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(owner.0.wait().unwrap().success());
    let result = rebuild_workspace(root.path(), &OperationContext::default()).unwrap();
    assert_eq!(result["selectedScopeApplied"], true, "{result}");
    assert_integrity(root.path(), 1);
}

#[test]
fn terminating_the_owner_process_releases_the_gate() {
    let root = fixture();
    let mut owner = Owner::start(root.path());
    let context = OperationContext::with_timeout(Duration::from_millis(400));
    assert_eq!(sync_workspace_all(root.path(), &context).unwrap_err().0, "TIMEOUT");
    owner.0.kill().unwrap();
    owner.0.wait().unwrap();
    let result = sync_workspace_all(root.path(), &OperationContext::default()).unwrap();
    assert_eq!(result["selectedScopeApplied"], true, "{result}");
    assert_integrity(root.path(), 1);
}
