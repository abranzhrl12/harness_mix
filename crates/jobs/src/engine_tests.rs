use super::execute::retryable;
use super::*;

#[test]
fn job_service_event_adapter_preserves_events_lag_and_close() {
    assert!(matches!(
        super::map_task_event(Ok(json!({"kind":"state","taskId":"t1"}))),
        harness_application::ports::JobEvent::Event(value) if value["taskId"] == "t1"
    ));
    assert!(matches!(
        super::map_task_event(Err(tokio::sync::broadcast::error::RecvError::Lagged(4))),
        harness_application::ports::JobEvent::Lagged(4)
    ));
    assert!(matches!(
        super::map_task_event(Err(tokio::sync::broadcast::error::RecvError::Closed)),
        harness_application::ports::JobEvent::Closed
    ));
}
use harness_domain::process::StopReason;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{fs::File, path::Path};

type TestRun = dyn Fn(&OperationContext, &mut dyn FnMut(bool, &[u8])) -> Result<ProcessOutcome, Failure>
    + Send
    + Sync;
type TestPrepare =
    dyn Fn(&Path, &VerifyArgs, Option<Duration>) -> Result<(), Failure> + Send + Sync;

struct FakeExecutor {
    prepare: Arc<TestPrepare>,
    run: Arc<TestRun>,
}
struct FakePrepared(Arc<TestRun>);

impl VerificationExecutor for FakeExecutor {
    fn prepare(
        &self,
        root: &Path,
        args: &VerifyArgs,
        timeout: Option<Duration>,
    ) -> Result<Box<dyn PreparedVerification>, Failure> {
        (self.prepare)(root, args, timeout)?;
        Ok(Box::new(FakePrepared(self.run.clone())))
    }
}

impl PreparedVerification for FakePrepared {
    fn execute(
        &self,
        context: &OperationContext,
        observer: &mut dyn FnMut(bool, &[u8]),
    ) -> Result<ProcessOutcome, Failure> {
        (self.0)(context, observer)
    }
}

struct TestPinner;
impl DirectoryPinner for TestPinner {
    fn pin_directory(&self, root: &Path, path: &Path) -> Result<Vec<File>, Failure> {
        if !path.starts_with(root) {
            return Err(("TASK_STORE_PATH", "Unsafe background job storage path"));
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
                        .map_err(|_| ("TASK_STORE_PATH", "Unsafe background job storage path"))
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    File::options()
                        .read(true)
                        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                        .open(ancestor)
                        .map_err(|_| ("TASK_STORE_PATH", "Unsafe background job storage path"))
                }
                #[cfg(not(any(windows, unix)))]
                {
                    File::open(ancestor)
                        .map_err(|_| ("TASK_STORE_PATH", "Unsafe background job storage path"))
                }
            })
            .collect()
    }
}

fn create_manager(root: &Path) -> TaskManager {
    manager_with(root, |_, _, _| Ok(()), |_, _| Ok(success()))
}

fn manager_with(
    root: &Path,
    prepare: impl Fn(&Path, &VerifyArgs, Option<Duration>) -> Result<(), Failure>
    + Send
    + Sync
    + 'static,
    run: impl Fn(&OperationContext, &mut dyn FnMut(bool, &[u8])) -> Result<ProcessOutcome, Failure>
    + Send
    + Sync
    + 'static,
) -> TaskManager {
    TaskManager::new(
        root,
        Arc::new(FakeExecutor {
            prepare: Arc::new(prepare),
            run: Arc::new(run),
        }),
        Arc::new(TestPinner),
    )
    .unwrap()
}
fn args(value: Value) -> TaskArgs {
    serde_json::from_value(value).unwrap()
}
fn workspace() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    root
}
#[test]
fn snapshot_preserves_task_and_event_payloads() {
    let id = uuid::Uuid::new_v4().to_string();
    let mut state = State::default();
    state.tasks.push_back(Task {
        id: id.clone(),
        lifecycle: JobLifecycle::queued(),
        configuration: json!({"verify":{"language":"rust"},"timeoutSeconds":null,"maxAttempts":1}),
        result: Some(json!({"passed":true,"details":{"summary":"preserved"}})),
        context: OperationContext::without_deadline(),
        wake: Arc::new(Notify::new()),
    });
    state.journal.push_back(json!({
        "cursor":1,
        "taskId":id,
        "kind":"output",
        "data":{"stream":"stderr","text":"payload","textLossy":false}
    }));
    state.sequence = 1;

    let expected = json!({
        "tasks":state.tasks.iter().map(Task::value).collect::<Vec<_>>(),
        "events":state.journal,
        "cursor":state.sequence
    });
    assert_eq!(Inner::snapshot(&state), expected);
}

#[test]
#[ignore = "opt-in benchmark for the durable snapshot critical path"]
fn measure_task_snapshot_and_sqlite_save_at_retention_limits() {
    use std::time::Instant;

    let root = workspace();
    let ids: Vec<_> = (0..MAX_RETAINED)
        .map(|_| uuid::Uuid::new_v4().to_string())
        .collect();
    let mut state = State {
        tasks: ids
            .iter()
            .map(|id| Task {
                id: id.clone(),
                lifecycle: JobLifecycle::restore("succeeded", 1, 1).unwrap(),
                configuration: json!({"verify":{"language":"rust"},"timeoutSeconds":null,"maxAttempts":1}),
                result: Some(json!({"passed":true,"summary":"x".repeat(256)})),
                context: OperationContext::without_deadline(),
                wake: Arc::new(Notify::new()),
            })
            .collect(),
        journal: (1..=MAX_EVENTS)
            .map(|cursor| {
                json!({
                    "cursor":cursor,
                    "taskId":ids[cursor % ids.len()],
                    "kind":"output",
                    "data":{"stream":"stdout","text":"x".repeat(256),"textLossy":false}
                })
            })
            .collect(),
        sequence: MAX_EVENTS as u64,
        ..State::default()
    };
    state.store = Some(Store::create(root.path(), &TestPinner).unwrap());

    let legacy_snapshot = || {
        json!({
            "tasks":state.tasks.iter().map(Task::value).collect::<Vec<_>>(),
            "events":state.journal,
            "cursor":state.sequence
        })
    };
    let snapshot = Inner::snapshot(&state);
    assert_eq!(snapshot, legacy_snapshot());
    assert_eq!(snapshot["tasks"].as_array().unwrap().len(), MAX_RETAINED);
    assert_eq!(snapshot["events"].as_array().unwrap().len(), MAX_EVENTS);
    assert_eq!(snapshot["cursor"], MAX_EVENTS as u64);

    // Warm up SQLite and measure its persistence separately from snapshot creation.
    state.store.as_ref().unwrap().save(&snapshot).unwrap();
    let iterations = 40;
    let mut legacy_elapsed = Duration::ZERO;
    for _ in 0..iterations {
        let started = Instant::now();
        std::hint::black_box(legacy_snapshot());
        legacy_elapsed += started.elapsed();
    }
    let mut snapshot_elapsed = Duration::ZERO;
    for _ in 0..iterations {
        let started = Instant::now();
        std::hint::black_box(Inner::snapshot(&state));
        snapshot_elapsed += started.elapsed();
    }
    let mut save_elapsed = Duration::ZERO;
    for _ in 0..iterations {
        let started = Instant::now();
        state.store.as_ref().unwrap().save(&snapshot).unwrap();
        save_elapsed += started.elapsed();
    }

    eprintln!(
        "task-persistence benchmark: tasks={}, events={}, snapshot_bytes={}, iterations={}, legacy_snapshot_avg_us={}, optimized_snapshot_avg_us={}, sqlite_save_avg_us={}",
        MAX_RETAINED,
        MAX_EVENTS,
        snapshot.to_string().len(),
        iterations,
        legacy_elapsed.as_micros() / iterations,
        snapshot_elapsed.as_micros() / iterations,
        save_elapsed.as_micros() / iterations,
    );
}

#[tokio::test]
async fn queued_task_cancels_without_acquiring_execution_slot() {
    let root = workspace();
    let manager = create_manager(root.path());
    let _slots = manager
        .inner
        .permits
        .clone()
        .acquire_many_owned(2)
        .await
        .unwrap();
    let task = manager
        .handle(args(json!({"action":"start","verify":{"language":"rust"}})))
        .await
        .unwrap();
    let id = task["taskId"].as_str().unwrap();
    manager
        .handle(args(json!({"action":"cancel","taskId":id})))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let state = manager
                .handle(args(json!({"action":"status","taskId":id})))
                .await
                .unwrap();
            if state["status"] == "cancelled" {
                assert_eq!(state["attempt"], 0);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn queue_bound_and_shutdown_are_enforced() {
    let root = workspace();
    let manager = create_manager(root.path());
    let _slots = manager
        .inner
        .permits
        .clone()
        .acquire_many_owned(2)
        .await
        .unwrap();
    for _ in 0..MAX_ACTIVE {
        manager
            .handle(args(json!({"action":"start","verify":{"language":"rust"}})))
            .await
            .unwrap();
    }
    assert_eq!(
        manager
            .handle(args(json!({"action":"start","verify":{"language":"rust"}})))
            .await
            .unwrap_err()
            .0,
        "TASK_BUSY"
    );
    manager.shutdown();
    assert_eq!(
        manager
            .handle(args(json!({"action":"start","verify":{"language":"rust"}})))
            .await
            .unwrap_err()
            .0,
        "TASK_SHUTDOWN"
    );
    assert!(
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .tasks
            .iter()
            .all(|task| task.context.check().is_err())
    );
}
#[tokio::test]
async fn queued_plan_is_revalidated_before_launch() {
    let root = workspace();
    let manager = manager_with(
        root.path(),
        |root, _, _| {
            if root.join("Cargo.toml").is_file() {
                Ok(())
            } else {
                Err(("VERIFY_SCOPE", "Workspace manifest disappeared"))
            }
        },
        |_, _| Ok(success()),
    );
    let slots = manager
        .inner
        .permits
        .clone()
        .acquire_many_owned(2)
        .await
        .unwrap();
    let task = manager
        .handle(args(json!({"action":"start","verify":{"language":"rust"}})))
        .await
        .unwrap();
    std::fs::remove_file(root.path().join("Cargo.toml")).unwrap();
    drop(slots);
    let id = task["taskId"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let state = manager
                .handle(args(json!({"action":"status","taskId":id})))
                .await
                .unwrap();
            if state["status"] == "failed" {
                assert!(matches!(
                    state["result"]["error"].as_str(),
                    Some("VERIFY_SCOPE" | "PATH")
                ));
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn event_journal_replays_and_reports_evicted_cursors() {
    let root = workspace();
    let manager = create_manager(root.path());
    let mut receiver = manager.subscribe();
    manager.inner.output("fixture", false, b"first");
    assert_eq!(receiver.recv().await.unwrap()["cursor"], 1);
    for _ in 0..MAX_EVENTS {
        manager.inner.output("fixture", true, b"next");
    }
    let events = manager
        .handle(args(json!({"action":"events","after":0})))
        .await
        .unwrap();
    assert_eq!(events["events"].as_array().unwrap().len(), 32);
    assert_eq!(events["gap"], true);
    assert_eq!(events["cursor"], 33);
    assert_eq!(events["latestCursor"], (MAX_EVENTS + 1) as u64);
    assert_eq!(events["hasMore"], true);
}
#[test]
fn retries_only_prelaunch_transient_failure_with_remaining_attempts() {
    assert!(retryable("PROCESS_SPAWN_TRANSIENT", 1, 3));
    assert!(!retryable("PROCESS_SPAWN_TRANSIENT", 3, 3));
    for code in [
        "PROCESS_SPAWN",
        "PROCESS_PIPE",
        "PROCESS_WAIT",
        "CANCELLED",
        "TIMEOUT",
        "BUILD_FAILED",
    ] {
        assert!(!retryable(code, 1, 3));
    }
}
fn success() -> ProcessOutcome {
    ProcessOutcome {
        exit_code: Some(0),
        stdout: vec![],
        stderr: vec![],
        elapsed: Duration::ZERO,
        reason: StopReason::Exited,
        tree_termination_confirmed: true,
    }
}
async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn scheduler_runs_two_and_keeps_third_queued() {
    let root = workspace();
    let running = Arc::new(AtomicUsize::new(0));
    let count = running.clone();
    let manager = manager_with(
        root.path(),
        |_, _, _| Ok(()),
        move |context, _| {
            count.fetch_add(1, Ordering::SeqCst);
            while context.check().is_ok() {
                std::thread::sleep(Duration::from_millis(2));
            }
            count.fetch_sub(1, Ordering::SeqCst);
            Ok(ProcessOutcome {
                reason: StopReason::Cancelled,
                ..success()
            })
        },
    );
    let mut ids = Vec::new();
    for _ in 0..3 {
        ids.push(
            manager
                .handle(args(json!({"action":"start","verify":{"language":"rust"}})))
                .await
                .unwrap()["taskId"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    until(|| running.load(Ordering::SeqCst) == 2).await;
    assert_eq!(
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .tasks
            .iter()
            .filter(|t| t.lifecycle.status() == JobStatus::Queued)
            .count(),
        1
    );
    assert_eq!(manager.inner.permits.available_permits(), 0);
    manager.shutdown();
    until(|| {
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .tasks
            .iter()
            .all(|t| terminal(t.lifecycle.status()))
    })
    .await;
    assert_eq!(running.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn shutdown_drains_workers_and_persists_terminal_state_before_restart() {
    let root = workspace();
    let running = Arc::new(AtomicUsize::new(0));
    let count = running.clone();
    let manager = manager_with(
        root.path(),
        |_, _, _| Ok(()),
        move |context, _| {
            count.fetch_add(1, Ordering::SeqCst);
            while context.check().is_ok() {
                std::thread::sleep(Duration::from_millis(2));
            }
            // Model a real runner that must reap children after cancellation.
            std::thread::sleep(Duration::from_millis(30));
            count.fetch_sub(1, Ordering::SeqCst);
            Ok(ProcessOutcome {
                reason: StopReason::Cancelled,
                ..success()
            })
        },
    );
    for _ in 0..3 {
        manager
            .handle(args(json!({"action":"start","verify":{"language":"rust"}})))
            .await
            .unwrap();
    }
    until(|| running.load(Ordering::SeqCst) == 2).await;
    // Cloned request handlers may be dropped without stopping the service.
    drop(manager.clone());
    assert!(
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .tasks
            .iter()
            .all(|t| t.context.check().is_ok())
    );
    assert!(manager.shutdown_and_wait(Duration::from_secs(3)).await);
    assert_eq!(running.load(Ordering::SeqCst), 0);
    assert_eq!(manager.inner.workers.load(Ordering::SeqCst), 0);
    assert!(
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .tasks
            .iter()
            .all(|t| t.lifecycle.status() == JobStatus::Cancelled)
    );
    drop(manager);
    let manager = create_manager(root.path());
    let state = manager.inner.state.lock().unwrap();
    assert!(
        state.persistence_error.is_none(),
        "writer lock must be released"
    );
    assert_eq!(state.tasks.len(), 3);
    assert!(
        state
            .tasks
            .iter()
            .all(|t| t.lifecycle.status() == JobStatus::Cancelled)
    );
}
#[tokio::test]
async fn dropping_last_handler_cancels_active_job_and_releases_writer() {
    let root = workspace();
    let running = Arc::new(AtomicUsize::new(0));
    let count = running.clone();
    let manager = manager_with(
        root.path(),
        |_, _, _| Ok(()),
        move |context, _| {
            count.store(1, Ordering::SeqCst);
            while context.check().is_ok() {
                std::thread::sleep(Duration::from_millis(2));
            }
            count.store(0, Ordering::SeqCst);
            Ok(ProcessOutcome {
                reason: StopReason::Cancelled,
                ..success()
            })
        },
    );
    manager
        .handle(args(json!({"action":"start","verify":{"language":"rust"}})))
        .await
        .unwrap();
    until(|| running.load(Ordering::SeqCst) == 1).await;
    let inner = Arc::downgrade(&manager.inner);
    drop(manager);
    until(|| inner.upgrade().is_none()).await;
    assert_eq!(running.load(Ordering::SeqCst), 0);
    let manager = create_manager(root.path());
    let state = manager.inner.state.lock().unwrap();
    assert!(state.persistence_error.is_none());
    assert_eq!(
        state.tasks.front().unwrap().lifecycle.status(),
        JobStatus::Cancelled
    );
}
#[tokio::test]
async fn transient_prelaunch_failures_retry_then_succeed() {
    let root = workspace();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let manager = manager_with(
        root.path(),
        |_, _, _| Ok(()),
        move |_, _| {
            if count.fetch_add(1, Ordering::SeqCst) < 2 {
                Err(("PROCESS_SPAWN_TRANSIENT", "fixture busy"))
            } else {
                Ok(success())
            }
        },
    );
    manager
        .handle(args(
            json!({"action":"start","verify":{"language":"rust"},"maxAttempts":3}),
        ))
        .await
        .unwrap();
    until(|| {
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .tasks
            .front()
            .unwrap()
            .lifecycle
            .status()
            == JobStatus::Succeeded
    })
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .tasks
            .front()
            .unwrap()
            .lifecycle
            .attempt(),
        3
    );
}
#[tokio::test]
async fn cancellation_interrupts_retry_backoff() {
    let root = workspace();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let manager = manager_with(
        root.path(),
        |_, _, _| Ok(()),
        move |_, _| {
            count.fetch_add(1, Ordering::SeqCst);
            Err(("PROCESS_SPAWN_TRANSIENT", "fixture busy"))
        },
    );
    let task = manager
        .handle(args(
            json!({"action":"start","verify":{"language":"rust"},"maxAttempts":3}),
        ))
        .await
        .unwrap();
    until(|| {
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .tasks
            .front()
            .unwrap()
            .lifecycle
            .status()
            == JobStatus::RetryWait
    })
    .await;
    manager
        .handle(args(json!({"action":"cancel","taskId":task["taskId"]})))
        .await
        .unwrap();
    until(|| {
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .tasks
            .front()
            .unwrap()
            .lifecycle
            .status()
            == JobStatus::Cancelled
    })
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn new_and_read_queries_do_not_create_job_storage() {
    let root = workspace();
    let before = std::fs::read_dir(root.path()).unwrap().count();
    let manager = create_manager(root.path());
    manager
        .handle(args(json!({"action":"list"})))
        .await
        .unwrap();
    manager
        .handle(args(json!({"action":"events"})))
        .await
        .unwrap();
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), before);
}
#[tokio::test]
async fn completed_job_and_cursor_survive_restart() {
    let root = workspace();
    let manager = manager_with(
        root.path(),
        |_, _, _| Ok(()),
        |_, observer| {
            observer(false, b"persisted output");
            Ok(success())
        },
    );
    let task = manager
        .handle(args(json!({"action":"start","verify":{"language":"rust"}})))
        .await
        .unwrap();
    until(|| {
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .tasks
            .front()
            .unwrap()
            .lifecycle
            .status()
            == JobStatus::Succeeded
            && Arc::strong_count(&manager.inner) == 1
    })
    .await;
    let cursor = manager.inner.state.lock().unwrap().sequence;
    drop(manager);
    let manager = create_manager(root.path());
    let restored = manager
        .handle(args(json!({"action":"status","taskId":task["taskId"]})))
        .await
        .unwrap();
    assert_eq!(restored["status"], "succeeded");
    assert!(restored["persistenceError"].is_null());
    let query: harness_domain::contracts::ProcessStatusArgs =
        serde_json::from_value(json!({"runId":task["taskId"],"list":"recent","limit":1})).unwrap();
    let process_status = manager.process_status(&query).unwrap();
    assert_eq!(process_status["run"]["state"], "succeeded");
    assert_eq!(process_status["run"]["result"]["passed"], true);
    assert_eq!(process_status["runs"].as_array().unwrap().len(), 1);
    let active: harness_domain::contracts::ProcessStatusArgs =
        serde_json::from_value(json!({"list":"active"})).unwrap();
    assert!(
        manager.process_status(&active).unwrap()["runs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(manager.inner.state.lock().unwrap().sequence, cursor);
    assert!(
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .journal
            .iter()
            .any(|e| e["kind"] == "output")
    );
}
#[tokio::test]
async fn recovery_marks_active_jobs_interrupted_and_persists_event() {
    let root = workspace();
    let id = uuid::Uuid::new_v4().to_string();
    let store = Store::create(root.path(), &TestPinner).unwrap();
    store.save(&json!({"tasks":[{"taskId":id,"status":"running","attempt":1,"configuration":{"verify":{"language":"rust"}},"result":null}],"events":[],"cursor":12})).unwrap();
    drop(store);
    let manager = create_manager(root.path());
    let status = manager
        .handle(args(json!({"action":"status","taskId":id})))
        .await
        .unwrap();
    assert_eq!(status["status"], "interrupted");
    assert_eq!(status["attempt"], 1);
    assert_eq!(manager.inner.state.lock().unwrap().sequence, 13);
    drop(manager);
    let manager = create_manager(root.path());
    assert_eq!(
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .tasks
            .front()
            .unwrap()
            .lifecycle
            .status(),
        JobStatus::Interrupted
    );
    assert_eq!(manager.inner.state.lock().unwrap().sequence, 13);
}
#[tokio::test]
async fn writer_conflict_disables_jobs_without_failing_manager_creation() {
    let root = workspace();
    let store = Store::create(root.path(), &TestPinner).unwrap();
    let manager = create_manager(root.path());
    let list = manager
        .handle(args(json!({"action":"list"})))
        .await
        .unwrap();
    assert!(!list["persistenceError"].is_null());
    assert!(
        manager
            .handle(args(json!({"action":"start","verify":{"language":"rust"}})))
            .await
            .is_err()
    );
    drop(store);
    // Once the previous writer releases the lock, lazy recovery clears TASK_STORE_BUSY
    let list_after = manager
        .handle(args(json!({"action":"list"})))
        .await
        .unwrap();
    assert!(list_after["persistenceError"].is_null(), "{list_after}");
    let diag = manager.diagnostics();
    assert!(diag["persistenceError"].is_null(), "{diag}");
}
#[tokio::test]
async fn failed_commit_cancels_work_and_does_not_publish_or_advance_cursor() {
    let root = workspace();
    let manager = create_manager(root.path());
    let _slots = manager
        .inner
        .permits
        .clone()
        .acquire_many_owned(2)
        .await
        .unwrap();
    let task = manager
        .handle(args(json!({"action":"start","verify":{"language":"rust"}})))
        .await
        .unwrap();
    let mut receiver = manager.subscribe();
    let cursor;
    {
        let state = manager.inner.state.lock().unwrap();
        cursor = state.sequence;
        state.store.as_ref().unwrap().fail_writes();
    }
    assert!(!manager.inner.update(
        task["taskId"].as_str().unwrap(),
        JobStatus::Running,
        1,
        None,
    ));
    {
        let state = manager.inner.state.lock().unwrap();
        assert!(state.persistence_error.is_some());
        assert_eq!(state.sequence, cursor);
        assert!(state.tasks.front().unwrap().context.check().is_err());
    }
    let notice = receiver.try_recv().unwrap();
    assert_eq!(notice["kind"], "persistence_error");
    assert_eq!(notice["durable"], false);
    assert!(notice.get("cursor").is_none());
    assert!(matches!(
        receiver.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    assert!(
        manager
            .handle(args(json!({"action":"start","verify":{"language":"rust"}})))
            .await
            .is_err()
    );
}
