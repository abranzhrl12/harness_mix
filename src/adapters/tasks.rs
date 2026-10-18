//! Server-scoped background verification with bounded concurrency and replayable events.
use super::{process::NativeProcessRunner, verify};
use harness_domain::{
    Failure,
    contracts::VerifyArgs,
    execution::OperationContext,
    process::{ProcessOutcome, ProcessSpec, StopReason},
    tasks::TaskArgs,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Notify, Semaphore, broadcast};

const MAX_ACTIVE: usize = 32;
const MAX_RETAINED: usize = 64;
const MAX_EVENTS: usize = 256;
type Executor = dyn Fn(
        &Path,
        &ProcessSpec,
        &OperationContext,
        &mut dyn FnMut(bool, &[u8]),
    ) -> Result<ProcessOutcome, Failure>
    + Send
    + Sync;

#[derive(Clone)]
pub struct TaskManager {
    inner: Arc<Inner>,
}
struct Inner {
    root: PathBuf,
    state: Mutex<State>,
    permits: Arc<Semaphore>,
    events: broadcast::Sender<Value>,
    executor: Arc<Executor>,
}
#[derive(Default)]
struct State {
    tasks: VecDeque<Task>,
    journal: VecDeque<Value>,
    sequence: u64,
    closed: bool,
}
struct Task {
    id: String,
    status: &'static str,
    attempt: u8,
    context: OperationContext,
    wake: Arc<Notify>,
    result: Option<Value>,
    configuration: Value,
}
fn terminal(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled" | "timed_out")
}
impl Task {
    fn value(&self) -> Value {
        json!({"taskId":self.id,"status":self.status,"attempt":self.attempt,"configuration":self.configuration,"result":self.result})
    }
}
impl Inner {
    fn emit_locked(&self, state: &mut State, id: &str, kind: &str, data: Value) {
        state.sequence += 1;
        let event = json!({"cursor":state.sequence,"taskId":id,"kind":kind,"data":data});
        if state.journal.len() == MAX_EVENTS {
            state.journal.pop_front();
        }
        state.journal.push_back(event.clone());
        let _ = self.events.send(event);
    }
    fn update(&self, id: &str, status: &'static str, attempt: u8, result: Option<Value>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(task) = state.tasks.iter_mut().find(|t| t.id == id) {
            task.status = status;
            task.attempt = attempt;
            task.result = result;
            let data = task.value();
            self.emit_locked(&mut state, id, "state", data);
        }
    }
    fn output(&self, id: &str, stderr: bool, bytes: &[u8]) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Output is bounded by the runner; each event is capped to one pipe read.
        self.emit_locked(&mut state, id, "output", json!({"stream":if stderr {"stderr"} else {"stdout"},"text":String::from_utf8_lossy(bytes),"textLossy":std::str::from_utf8(bytes).is_err()}));
    }
}
impl TaskManager {
    pub fn new(root: &Path) -> std::io::Result<Self> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Workspace must be a directory",
            ));
        }
        let (events, _) = broadcast::channel(MAX_EVENTS);
        Ok(Self {
            inner: Arc::new(Inner {
                root,
                state: Mutex::new(State::default()),
                permits: Arc::new(Semaphore::new(2)),
                events,
                executor: Arc::new(|root, spec, context, observer| {
                    NativeProcessRunner::new(root)?.run_observed(spec, context, observer)
                }),
            }),
        })
    }
    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.inner.events.subscribe()
    }
    pub fn shutdown(&self) {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        for task in &state.tasks {
            if !terminal(task.status) {
                task.context.cancel();
                task.wake.notify_one();
            }
        }
    }
    pub async fn handle(&self, args: TaskArgs) -> Result<Value, Failure> {
        if args.action != "start"
            && (args.verify.is_some()
                || args.timeout_seconds.is_some()
                || args.max_attempts.is_some())
        {
            return Err((
                "ARGUMENT",
                "verify, timeoutSeconds and maxAttempts require start",
            ));
        }
        if args.action != "events" && args.after.is_some() {
            return Err(("ARGUMENT", "after requires events"));
        }
        if matches!(args.action.as_str(), "start" | "list") && args.task_id.is_some() {
            return Err(("ARGUMENT", "taskId is not valid for start or list"));
        }
        match args.action.as_str() {
            "start" => self.start(args),
            "list" => {
                let state = self
                    .inner
                    .state
                    .lock()
                    .map_err(|_| ("INTERNAL", "Task registry unavailable"))?;
                Ok(
                    json!({"tasks":state.tasks.iter().map(|t| json!({"taskId":t.id,"status":t.status,"attempt":t.attempt})).collect::<Vec<_>>(),"cursor":state.sequence,"concurrency":2}),
                )
            }
            "events" => {
                let state = self
                    .inner
                    .state
                    .lock()
                    .map_err(|_| ("INTERNAL", "Task registry unavailable"))?;
                let after = args.after.unwrap_or(0);
                let oldest = state
                    .journal
                    .front()
                    .and_then(|e| e["cursor"].as_u64())
                    .unwrap_or(state.sequence + 1);
                let events: Vec<_> = state
                    .journal
                    .iter()
                    .filter(|e| {
                        e["cursor"].as_u64().unwrap_or(0) > after
                            && args.task_id.as_ref().is_none_or(|id| e["taskId"] == *id)
                    })
                    .take(32)
                    .collect();
                let cursor = events
                    .last()
                    .and_then(|e| e["cursor"].as_u64())
                    .unwrap_or(state.sequence);
                Ok(
                    json!({"events":events,"cursor":cursor,"latestCursor":state.sequence,"hasMore":state.journal.iter().any(|e| e["cursor"].as_u64().unwrap_or(0)>cursor && args.task_id.as_ref().is_none_or(|id| e["taskId"] == *id)),"oldestCursor":oldest,"gap":after.saturating_add(1) < oldest}),
                )
            }
            "status" | "cancel" => {
                let id = args
                    .task_id
                    .as_deref()
                    .ok_or(("ARGUMENT", "taskId required"))?;
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .map_err(|_| ("INTERNAL", "Task registry unavailable"))?;
                let task = state
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == id)
                    .ok_or(("TASK_NOT_FOUND", "Task not found or expired"))?;
                if args.action == "cancel" && !terminal(task.status) {
                    task.context.cancel();
                    task.wake.notify_one();
                    let value = task.value();
                    self.inner
                        .emit_locked(&mut state, id, "cancel_requested", value);
                }
                let task = state.tasks.iter().find(|t| t.id == id).unwrap();
                Ok(task.value())
            }
            _ => Err(("ARGUMENT", "Unknown background task action")),
        }
    }
    fn start(&self, args: TaskArgs) -> Result<Value, Failure> {
        let verify_args = args.verify.ok_or(("ARGUMENT", "start requires verify"))?;
        let attempts = args.max_attempts.unwrap_or(1);
        if !(1..=3).contains(&attempts) {
            return Err(("ARGUMENT", "maxAttempts must be 1 to 3"));
        }
        let timeout = args.timeout_seconds.map(Duration::from_secs);
        if timeout.is_some_and(|d| d.is_zero()) {
            return Err(("ARGUMENT", "timeoutSeconds must be positive"));
        }
        verify::background_plan(&self.inner.root, &verify_args, timeout)?;
        let context = OperationContext::without_deadline();
        let wake = Arc::new(Notify::new());
        let id = uuid::Uuid::new_v4().to_string();
        let response;
        {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| ("INTERNAL", "Task registry unavailable"))?;
            if state.closed {
                return Err(("TASK_SHUTDOWN", "Task service is shutting down"));
            }
            if state.tasks.iter().filter(|t| !terminal(t.status)).count() >= MAX_ACTIVE {
                return Err(("TASK_BUSY", "Background task queue is full"));
            }
            while state.tasks.len() >= MAX_RETAINED {
                if let Some(i) = state.tasks.iter().position(|t| terminal(t.status)) {
                    state.tasks.remove(i);
                } else {
                    break;
                }
            }
            let configuration = json!({"verify":verify_args,"timeoutSeconds":args.timeout_seconds,"maxAttempts":attempts});
            let task = Task {
                id: id.clone(),
                status: "queued",
                attempt: 0,
                context: context.clone(),
                wake: wake.clone(),
                result: None,
                configuration,
            };
            response = task.value();
            state.tasks.push_back(task);
            self.inner
                .emit_locked(&mut state, &id, "state", response.clone());
        }
        let inner = self.inner.clone();
        tokio::spawn(async move {
            execute(inner, id, verify_args, timeout, context, wake, attempts).await;
        });
        Ok(response)
    }
}

fn retryable(code: &str, attempt: u8, maximum: u8) -> bool {
    code == "PROCESS_SPAWN_TRANSIENT" && attempt < maximum
}

async fn execute(
    inner: Arc<Inner>,
    id: String,
    verify_args: VerifyArgs,
    timeout: Option<Duration>,
    context: OperationContext,
    wake: Arc<Notify>,
    attempts: u8,
) {
    let permit = tokio::select! {
        permit = inner.permits.clone().acquire_owned() => permit.ok(),
        _ = wake.notified() => None,
    };
    if permit.is_none() || context.check().is_err() {
        inner.update(&id, "cancelled", 0, None);
        return;
    }
    for attempt in 1..=attempts {
        if context.check().is_err() {
            inner.update(&id, "cancelled", attempt - 1, None);
            return;
        }
        let spec = match verify::background_plan(&inner.root, &verify_args, timeout) {
            Ok(spec) => spec,
            Err((code, message)) => {
                inner.update(
                    &id,
                    "failed",
                    attempt,
                    Some(json!({"error":code,"message":message})),
                );
                return;
            }
        };
        inner.update(&id, "running", attempt, None);
        let work_inner = inner.clone();
        let work_id = id.clone();
        let work_context = context.clone();
        let result = tokio::task::spawn_blocking(move || {
            (work_inner.executor)(
                &work_inner.root,
                &spec,
                &work_context,
                &mut |stderr, bytes| work_inner.output(&work_id, stderr, bytes),
            )
        })
        .await;
        match result {
            Ok(Ok(outcome)) => {
                let status = match outcome.reason {
                    StopReason::Cancelled => "cancelled",
                    StopReason::TimedOut => "timed_out",
                    StopReason::Exited if outcome.exit_code == Some(0) => "succeeded",
                    _ => "failed",
                };
                let output = format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&outcome.stdout),
                    String::from_utf8_lossy(&outcome.stderr)
                );
                let result = json!({"passed":status == "succeeded","code":outcome.exit_code,"milliseconds":outcome.elapsed.as_millis() as u64,"output":output.chars().take(12000).collect::<String>(),"outputTruncated":output.chars().count()>12000,"limited":outcome.reason == StopReason::OutputLimit,"treeTerminationConfirmed":outcome.tree_termination_confirmed,"verificationFreshness":"not_tracked"});
                inner.update(&id, status, attempt, Some(result));
                return;
            }
            Ok(Err((code, _))) if retryable(code, attempt, attempts) => {
                inner.update(&id, "retry_wait", attempt, None);
                tokio::select! { _ = tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))) => {}, _ = wake.notified() => {} }
            }
            Ok(Err((code, message))) => {
                inner.update(
                    &id,
                    if context.check().is_err() {
                        "cancelled"
                    } else {
                        "failed"
                    },
                    attempt,
                    Some(json!({"error":code,"message":message})),
                );
                return;
            }
            Err(_) => {
                inner.update(
                    &id,
                    "failed",
                    attempt,
                    Some(json!({"error":"TASK_WORKER","message":"Background worker failed"})),
                );
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                .filter(|t| t.status == "queued")
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
                .all(|t| terminal(t.status))
        })
        .await;
        assert_eq!(running.load(Ordering::SeqCst), 0);
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
                .status
                == "succeeded"
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
                .attempt,
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
                .status
                == "retry_wait"
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
                .status
                == "cancelled"
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
