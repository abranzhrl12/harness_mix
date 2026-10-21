use super::execute::retryable;
use super::*;
use harness_domain::process::StopReason;
use std::sync::atomic::{AtomicUsize, Ordering};
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
#[tokio::test]
async fn queued_task_cancels_without_acquiring_execution_slot() {
    let root = workspace();
    let manager = TaskManager::new(root.path()).unwrap();
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
    let manager = TaskManager::new(root.path()).unwrap();
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
    let manager = TaskManager::new(root.path()).unwrap();
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
    let manager = TaskManager::new(root.path()).unwrap();
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
    let mut manager = TaskManager::new(root.path()).unwrap();
    let running = Arc::new(AtomicUsize::new(0));
    let count = running.clone();
    Arc::get_mut(&mut manager.inner).unwrap().executor = Arc::new(move |_, _, context, _| {
        count.fetch_add(1, Ordering::SeqCst);
        while context.check().is_ok() {
            std::thread::sleep(Duration::from_millis(2));
        }
        count.fetch_sub(1, Ordering::SeqCst);
        Ok(ProcessOutcome {
            reason: StopReason::Cancelled,
            ..success()
        })
    });
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
    let mut manager = TaskManager::new(root.path()).unwrap();
    let running = Arc::new(AtomicUsize::new(0));
    let count = running.clone();
    Arc::get_mut(&mut manager.inner).unwrap().executor = Arc::new(move |_, _, context, _| {
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
    });
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
    let manager = TaskManager::new(root.path()).unwrap();
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
    let mut manager = TaskManager::new(root.path()).unwrap();
    let running = Arc::new(AtomicUsize::new(0));
    let count = running.clone();
    Arc::get_mut(&mut manager.inner).unwrap().executor = Arc::new(move |_, _, context, _| {
        count.store(1, Ordering::SeqCst);
        while context.check().is_ok() {
            std::thread::sleep(Duration::from_millis(2));
        }
        count.store(0, Ordering::SeqCst);
        Ok(ProcessOutcome {
            reason: StopReason::Cancelled,
            ..success()
        })
    });
    manager
        .handle(args(json!({"action":"start","verify":{"language":"rust"}})))
        .await
        .unwrap();
    until(|| running.load(Ordering::SeqCst) == 1).await;
    let inner = Arc::downgrade(&manager.inner);
    drop(manager);
    until(|| inner.upgrade().is_none()).await;
    assert_eq!(running.load(Ordering::SeqCst), 0);
    let manager = TaskManager::new(root.path()).unwrap();
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
    let mut manager = TaskManager::new(root.path()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    Arc::get_mut(&mut manager.inner).unwrap().executor = Arc::new(move |_, _, _, _| {
        if count.fetch_add(1, Ordering::SeqCst) < 2 {
            Err(("PROCESS_SPAWN_TRANSIENT", "fixture busy"))
        } else {
            Ok(success())
        }
    });
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
    let mut manager = TaskManager::new(root.path()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    Arc::get_mut(&mut manager.inner).unwrap().executor = Arc::new(move |_, _, _, _| {
        count.fetch_add(1, Ordering::SeqCst);
        Err(("PROCESS_SPAWN_TRANSIENT", "fixture busy"))
    });
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
    let manager = TaskManager::new(root.path()).unwrap();
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
    let mut manager = TaskManager::new(root.path()).unwrap();
    Arc::get_mut(&mut manager.inner).unwrap().executor = Arc::new(|_, _, _, observer| {
        observer(false, b"persisted output");
        Ok(success())
    });
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
    let manager = TaskManager::new(root.path()).unwrap();
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
    let store = Store::create(root.path()).unwrap();
    store.save(&json!({"tasks":[{"taskId":id,"status":"running","attempt":1,"configuration":{"verify":{"language":"rust"}},"result":null}],"events":[],"cursor":12})).unwrap();
    drop(store);
    let manager = TaskManager::new(root.path()).unwrap();
    let status = manager
        .handle(args(json!({"action":"status","taskId":id})))
        .await
        .unwrap();
    assert_eq!(status["status"], "interrupted");
    assert_eq!(status["attempt"], 1);
    assert_eq!(manager.inner.state.lock().unwrap().sequence, 13);
    drop(manager);
    let manager = TaskManager::new(root.path()).unwrap();
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
    let store = Store::create(root.path()).unwrap();
    let manager = TaskManager::new(root.path()).unwrap();
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
}
#[tokio::test]
async fn failed_commit_cancels_work_and_does_not_publish_or_advance_cursor() {
    let root = workspace();
    let manager = TaskManager::new(root.path()).unwrap();
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
#[tokio::test]
async fn native_typescript_jobs_parse_syntax_without_executing_code() {
    let root = workspace();
    std::fs::write(root.path().join("valid.tsx"),"const value: number = 1; const view = <main>{value}</main>; throw new Error('must not execute');").unwrap();
    std::fs::write(root.path().join("invalid.ts"), "const value: = ;").unwrap();
    let mut manager = TaskManager::new(root.path()).unwrap();
    Arc::get_mut(&mut manager.inner).unwrap().executor =
        Arc::new(|_, _, _, _| panic!("TypeScript syntax verification must not launch Node"));
    let valid=manager.handle(args(json!({"action":"start","verify":{"language":"typescript","scope":"valid.tsx","action":"build"}}))).await.unwrap();
    until(|| {
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == valid["taskId"].as_str().unwrap())
            .is_some_and(|t| terminal(t.lifecycle.status()))
    })
    .await;
    let state = manager
        .handle(args(json!({"action":"status","taskId":valid["taskId"]})))
        .await
        .unwrap();
    assert_eq!(state["status"], "succeeded");
    assert_eq!(
        state["result"]["verificationMode"],
        "typescript_syntax_only"
    );
    assert_eq!(state["result"]["codeExecuted"], false);
    let invalid=manager.handle(args(json!({"action":"start","verify":{"language":"typescript","scope":"invalid.ts","action":"build"}}))).await.unwrap();
    until(|| {
        manager
            .inner
            .state
            .lock()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == invalid["taskId"].as_str().unwrap())
            .is_some_and(|t| terminal(t.lifecycle.status()))
    })
    .await;
    assert_eq!(
        manager
            .handle(args(json!({"action":"status","taskId":invalid["taskId"]})))
            .await
            .unwrap()["status"],
        "failed"
    );
    assert_eq!(manager.handle(args(json!({"action":"start","verify":{"language":"typescript","scope":"valid.tsx","action":"test"}}))).await.unwrap_err().0,"UNSUPPORTED");
}
