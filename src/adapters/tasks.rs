//! Durable background verification with bounded concurrency and replayable events.
#[path = "observe.rs"]
mod observe;
use super::{process::NativeProcessRunner, tasks_store::Store, verify};
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
    sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}},
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

pub struct TaskManager {
    inner: Arc<Inner>,
}
struct Inner {
    root: PathBuf,
    state: Mutex<State>,
    permits: Arc<Semaphore>,
    events: broadcast::Sender<Value>,
    executor: Arc<Executor>,
    owners: AtomicUsize,
    workers: AtomicUsize,
    idle: Notify,
}
impl Clone for TaskManager {
    fn clone(&self) -> Self {
        self.inner.owners.fetch_add(1, Ordering::Relaxed);
        Self { inner: self.inner.clone() }
    }
}
impl Drop for TaskManager {
    fn drop(&mut self) {
        // Workers own Inner directly, so they cannot keep the service alive after
        // its last client/handler is dropped.
        if self.inner.owners.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shutdown();
        }
    }
}
struct WorkerGuard(Arc<Inner>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.workers.fetch_sub(1, Ordering::AcqRel);
        self.0.idle.notify_waiters();
    }
}
#[derive(Default)]
struct State {
    tasks: VecDeque<Task>,
    journal: VecDeque<Value>,
    sequence: u64,
    closed: bool,
    store: Option<Store>,
    persistence_error: Option<Failure>,
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
    matches!(
        status,
        "succeeded" | "failed" | "cancelled" | "timed_out" | "interrupted"
    )
}
impl Task {
    fn value(&self) -> Value {
        json!({"taskId":self.id,"status":self.status,"attempt":self.attempt,"configuration":self.configuration,"result":self.result})
    }
}
fn persistence_error(state: &State) -> Value {
    state
        .persistence_error
        .map(|(code, message)| json!({"code":code,"message":message}))
        .unwrap_or(Value::Null)
}
fn restore(snapshot: Value, state: &mut State) -> Result<(), Failure> {
    let invalid = ("TASK_STORE_FORMAT", "Invalid persisted job state");
    state.sequence = snapshot["cursor"].as_u64().ok_or(invalid)?;
    state.journal = snapshot["events"]
        .as_array()
        .ok_or(invalid)?
        .iter()
        .cloned()
        .collect();
    let values = snapshot["tasks"].as_array().ok_or(invalid)?;
    if values.len() > MAX_RETAINED
        || state.journal.len() > MAX_EVENTS
        || state.sequence >= i64::MAX as u64
    {
        return Err(invalid);
    }
    let mut previous = 0;
    for event in &state.journal {
        let cursor = event["cursor"].as_u64().ok_or(invalid)?;
        if cursor <= previous || cursor > state.sequence {
            return Err(invalid);
        }
        previous = cursor;
    }
    let mut ids = std::collections::HashSet::new();
    for value in values {
        let status = match value["status"].as_str().ok_or(invalid)? {
            "queued" => "queued",
            "running" => "running",
            "retry_wait" => "retry_wait",
            "succeeded" => "succeeded",
            "failed" => "failed",
            "cancelled" => "cancelled",
            "timed_out" => "timed_out",
            "interrupted" => "interrupted",
            _ => return Err(invalid),
        };
        let attempt =
            u8::try_from(value["attempt"].as_u64().ok_or(invalid)?).map_err(|_| invalid)?;
        let id = value["taskId"].as_str().ok_or(invalid)?.to_string();
        if !harness_domain::execution::valid_request_id(&id)
            || attempt > 3
            || !value["configuration"].is_object()
            || !ids.insert(id.clone())
        {
            return Err(invalid);
        }
        serde_json::from_value::<VerifyArgs>(value["configuration"]["verify"].clone())
            .map_err(|_| invalid)?;
        let maximum = value["configuration"]["maxAttempts"].as_u64().unwrap_or(1);
        if !(1..=3).contains(&maximum) || u64::from(attempt) > maximum {
            return Err(invalid);
        }
        state.tasks.push_back(Task {
            id,
            status,
            attempt,
            configuration: value["configuration"].clone(),
            result: (!value["result"].is_null()).then(|| value["result"].clone()),
            context: OperationContext::without_deadline(),
            wake: Arc::new(Notify::new()),
        });
    }
    Ok(())
}
impl Inner {
    fn snapshot(state: &State) -> Value {
        json!({"tasks":state.tasks.iter().map(Task::value).collect::<Vec<_>>(),"events":state.journal,"cursor":state.sequence})
    }
    fn fail_persistence(state: &mut State, error: Failure) {
        state.persistence_error = Some(error);
        for task in &state.tasks {
            if !terminal(task.status) {
                task.context.cancel();
                task.wake.notify_one();
            }
        }
    }
    fn persist(state: &mut State) -> Result<(), Failure> {
        if let Some(error) = state.persistence_error {
            return Err(error);
        }
        let snapshot = Self::snapshot(state);
        if let Some(store) = &mut state.store {
            if let Err(error) = store.save(&snapshot) {
                Self::fail_persistence(state, error);
                return Err(error);
            }
        }
        Ok(())
    }
    fn emit_locked(
        &self,
        state: &mut State,
        id: &str,
        kind: &str,
        data: Value,
    ) -> Result<(), Failure> {
        if let Some(error) = state.persistence_error {
            return Err(error);
        }
        if state.sequence >= i64::MAX as u64 - 1 {
            let error = ("TASK_STORE_FORMAT", "Background job event cursor exhausted");
            Self::fail_persistence(state, error);
            return Err(error);
        }
        state.sequence += 1;
        let event = json!({"cursor":state.sequence,"taskId":id,"kind":kind,"data":data});
        let evicted = if state.journal.len() == MAX_EVENTS {
            state.journal.pop_front()
        } else {
            None
        };
        state.journal.push_back(event.clone());
        if let Err(error) = Self::persist(state) {
            state.journal.pop_back();
            if let Some(event) = evicted {
                state.journal.push_front(event);
            }
            state.sequence -= 1;
            // This control notice deliberately has no durable journal cursor.
            // Subscribers must consult status/list and see persistenceError.
            let _ = self.events.send(json!({"kind":"persistence_error","durable":false,"data":{"code":error.0,"message":error.1},"recovery":"Inspect harness_jobs status or list for persistenceError; restart after restoring writable storage"}));
            return Err(error);
        }
        let _ = self.events.send(event);
        Ok(())
    }
    fn update(&self, id: &str, status: &'static str, attempt: u8, result: Option<Value>) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(task) = state.tasks.iter_mut().find(|t| t.id == id) {
            task.status = status;
            task.attempt = attempt;
            task.result = result;
            let data = task.value();
            return self.emit_locked(&mut state, id, "state", data).is_ok();
        }
        false
    }
    fn output(&self, id: &str, stderr: bool, bytes: &[u8]) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Output is bounded by the runner; each event is capped to one pipe read.
        let _ = self.emit_locked(&mut state, id, "output", json!({"stream":if stderr {"stderr"} else {"stdout"},"text":String::from_utf8_lossy(bytes),"textLossy":std::str::from_utf8(bytes).is_err()}));
    }
}
impl TaskManager {
    pub fn diagnostics(&self) -> Value {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let read = state.store.as_ref().map(|store| store.load());
        let error = state.persistence_error.or_else(|| {
            read.as_ref()
                .and_then(|result| result.as_ref().err().copied())
        });
        json!({"status":if error.is_some(){"error"}else if state.store.is_some(){"ok"}else{"skipped"},"storage":if error.is_some(){"error"}else if state.store.is_some(){"ready"}else{"lazy"},"persistenceError":error.map(|(code,message)|json!({"code":code,"message":message})),"retained":state.tasks.len(),"active":state.tasks.iter().filter(|t|!terminal(t.status)).count(),"cursor":state.sequence,"schemaChecked":state.store.is_some()&&error.is_none(),"readChecked":matches!(read,Some(Ok(_)))&&error.is_none(),"writeabilityChecked":false})
    }
    /// Read retained managed jobs without creating or modifying the durable store.
    pub fn process_status(
        &self,
        args: &harness_domain::contracts::ProcessStatusArgs,
    ) -> Result<Value, Failure> {
        use harness_domain::contracts::ProcessListFilter;
        let limit = args.limit.unwrap_or(10);
        if !(1..=20).contains(&limit) {
            return Err(("ARGUMENT", "limit: 1–20"));
        }
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| ("INTERNAL", "Task registry unavailable"))?;
        let describe = |task: &Task| {
            let mut value = task.value();
            value["runId"] = json!(task.id);
            value["state"] = json!(task.status);
            value
        };
        let mut result =
            json!({"persistenceError":persistence_error(&state),"cursor":state.sequence});
        if let Some(id) = &args.run_id {
            if !harness_domain::execution::valid_request_id(id) {
                return Err(("ARGUMENT", "Invalid runId"));
            }
            result["run"] = state
                .tasks
                .iter()
                .find(|t| &t.id == id)
                .map(describe)
                .unwrap_or_else(
                    || json!({"runId":id,"state":"unknown","reason":"not_found_or_expired"}),
                );
        }
        if let Some(filter) = args.list {
            result["runs"] = json!(
                state
                    .tasks
                    .iter()
                    .rev()
                    .filter(|task| matches!(filter, ProcessListFilter::Recent)
                        || !terminal(task.status))
                    .take(limit)
                    .map(describe)
                    .collect::<Vec<_>>()
            );
        }
        Ok(result)
    }
    pub fn new(root: &Path) -> std::io::Result<Self> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Workspace must be a directory",
            ));
        }
        let (events, _) = broadcast::channel(MAX_EVENTS);
        let mut state = State::default();
        match Store::open_existing(&root) {
            Ok(Some(store)) => match store
                .load()
                .and_then(|snapshot| restore(snapshot, &mut state))
            {
                Ok(()) => {
                    state.store = Some(store);
                }
                Err(error) => {
                    state.tasks.clear();
                    state.journal.clear();
                    state.sequence = 0;
                    state.store = Some(store);
                    state.persistence_error = Some(error);
                }
            },
            Ok(None) => {}
            Err(error) => state.persistence_error = Some(error),
        }
        let manager = Self {
            inner: Arc::new(Inner {
                root,
                state: Mutex::new(state),
                permits: Arc::new(Semaphore::new(2)),
                events,
                executor: Arc::new(|root, spec, context, observer| {
                    NativeProcessRunner::new(root)?.run_observed(spec, context, observer)
                }),
                owners: AtomicUsize::new(1),
                workers: AtomicUsize::new(0),
                idle: Notify::new(),
            }),
        };
        {
            let mut state = manager
                .inner
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if state.persistence_error.is_none() {
                let ids: Vec<_> = state
                    .tasks
                    .iter()
                    .filter(|t| !terminal(t.status))
                    .map(|t| t.id.clone())
                    .collect();
                for id in ids {
                    let task = state.tasks.iter_mut().find(|t| t.id == id).unwrap();
                    task.status = "interrupted";
                    task.result = Some(
                        json!({"error":"TASK_INTERRUPTED","message":"Server restarted before task completion; task was not rerun"}),
                    );
                    let data = task.value();
                    if manager
                        .inner
                        .emit_locked(&mut state, &id, "state", data)
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
        Ok(manager)
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
    /// Cancel and drain workers before the async runtime is stopped. A false
    /// result means the caller's cleanup deadline elapsed, not task success.
    pub async fn shutdown_and_wait(&self, timeout: Duration) -> bool {
        self.shutdown();
        tokio::time::timeout(timeout, async {
            loop {
                let idle = self.inner.idle.notified();
                tokio::pin!(idle);
                idle.as_mut().enable();
                if self.inner.workers.load(Ordering::Acquire) == 0 {
                    return;
                }
                idle.await;
            }
        }).await.is_ok()
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
                    json!({"tasks":state.tasks.iter().map(|t| json!({"taskId":t.id,"status":t.status,"attempt":t.attempt})).collect::<Vec<_>>(),"cursor":state.sequence,"concurrency":2,"persistenceError":persistence_error(&state)}),
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
                    json!({"events":events,"cursor":cursor,"latestCursor":state.sequence,"hasMore":state.journal.iter().any(|e| e["cursor"].as_u64().unwrap_or(0)>cursor && args.task_id.as_ref().is_none_or(|id| e["taskId"] == *id)),"oldestCursor":oldest,"gap":after.saturating_add(1) < oldest,"persistenceError":persistence_error(&state)}),
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
                        .emit_locked(&mut state, id, "cancel_requested", value)?;
                }
                let task = state.tasks.iter().find(|t| t.id == id).unwrap();
                let mut value = task.value();
                value["persistenceError"] = persistence_error(&state);
                Ok(value)
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
            if let Some(error) = state.persistence_error {
                return Err(error);
            }
            if state.store.is_none() {
                match Store::create(&self.inner.root) {
                    Ok(store) => state.store = Some(store),
                    Err(error) => {
                        Inner::fail_persistence(&mut state, error);
                        return Err(error);
                    }
                }
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
                .emit_locked(&mut state, &id, "state", response.clone())?;
            self.inner.workers.fetch_add(1, Ordering::AcqRel);
        }
        let inner = self.inner.clone();
        let worker = WorkerGuard(inner.clone());
        tokio::spawn(async move {
            let _worker = worker;
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
    let verify_args = Arc::new(verify_args);
    let permit = tokio::select! {
        permit = inner.permits.clone().acquire_owned() => permit.ok(),
        _ = wake.notified() => None,
    };
    if permit.is_none() || context.check().is_err() {
        inner.update(&id, "cancelled", 0, None);
        return;
    }
    let verify_args = Arc::new(verify_args);
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
        if !inner.update(&id, "running", attempt, None) {
            return;
        }
        let work_inner = inner.clone();
        let work_id = id.clone();
        let work_context = context.with_execution_timeout(timeout);
        let work_args = verify_args.clone();
        let syntax_only = matches!(spec, verify::BackgroundVerificationPlan::TypeScriptSyntax);
        let result = tokio::task::spawn_blocking(move || match spec {
            verify::BackgroundVerificationPlan::Process(spec) => (work_inner.executor)(
                &work_inner.root,
                &spec,
                &work_context,
                &mut |stderr, bytes| work_inner.output(&work_id, stderr, bytes),
            ),
            verify::BackgroundVerificationPlan::TypeScriptSyntax => {
                let outcome =
                    verify::run_typescript_syntax(&work_inner.root, &work_args, &work_context)?;
                for chunk in outcome.stdout.chunks(8192) {
                    work_inner.output(&work_id, false, chunk);
                }
                for chunk in outcome.stderr.chunks(8192) {
                    work_inner.output(&work_id, true, chunk);
                }
                Ok(outcome)
            }
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
                let mut result = json!({"passed":status == "succeeded","code":outcome.exit_code,"milliseconds":outcome.elapsed.as_millis() as u64,"output":output.chars().take(12000).collect::<String>(),"outputTruncated":output.chars().count()>12000,"limited":outcome.reason == StopReason::OutputLimit,"treeTerminationConfirmed":outcome.tree_termination_confirmed,"verificationFreshness":"not_tracked"});
                if syntax_only {
                    result["language"] = json!("typescript");
                    result["verificationMode"] = json!("typescript_syntax_only");
                    result["typeCheckPerformed"] = json!(false);
                    result["codeExecuted"] = json!(false);
                    result["testCountVerified"] = json!(false);
                }
                inner.update(&id, status, attempt, Some(result));
                return;
            }
            Ok(Err((code, _))) if retryable(code, attempt, attempts) => {
                if !inner.update(&id, "retry_wait", attempt, None) {
                    return;
                }
                tokio::select! { _ = tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))) => {}, _ = wake.notified() => {} }
            }
            Ok(Err((code, message))) => {
                inner.update(
                    &id,
                    if code == "TIMEOUT" {
                        "timed_out"
                    } else if code == "CANCELLED" || context.check().is_err() {
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
            Ok(ProcessOutcome { reason: StopReason::Cancelled, ..success() })
        });
        for _ in 0..3 {
            manager.handle(args(json!({"action":"start","verify":{"language":"rust"}}))).await.unwrap();
        }
        until(|| running.load(Ordering::SeqCst) == 2).await;
        // Cloned request handlers may be dropped without stopping the service.
        drop(manager.clone());
        assert!(manager.inner.state.lock().unwrap().tasks.iter().all(|t| t.context.check().is_ok()));
        assert!(manager.shutdown_and_wait(Duration::from_secs(3)).await);
        assert_eq!(running.load(Ordering::SeqCst), 0);
        assert_eq!(manager.inner.workers.load(Ordering::SeqCst), 0);
        assert!(manager.inner.state.lock().unwrap().tasks.iter().all(|t| t.status == "cancelled"));
        drop(manager);
        let manager = TaskManager::new(root.path()).unwrap();
        let state = manager.inner.state.lock().unwrap();
        assert!(state.persistence_error.is_none(), "writer lock must be released");
        assert_eq!(state.tasks.len(), 3);
        assert!(state.tasks.iter().all(|t| t.status == "cancelled"));
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
            Ok(ProcessOutcome { reason: StopReason::Cancelled, ..success() })
        });
        manager.handle(args(json!({"action":"start","verify":{"language":"rust"}}))).await.unwrap();
        until(|| running.load(Ordering::SeqCst) == 1).await;
        let inner = Arc::downgrade(&manager.inner);
        drop(manager);
        until(|| inner.upgrade().is_none()).await;
        assert_eq!(running.load(Ordering::SeqCst), 0);
        let manager = TaskManager::new(root.path()).unwrap();
        let state = manager.inner.state.lock().unwrap();
        assert!(state.persistence_error.is_none());
        assert_eq!(state.tasks.front().unwrap().status, "cancelled");
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
                .status
                == "succeeded"
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
            serde_json::from_value(json!({"runId":task["taskId"],"list":"recent","limit":1}))
                .unwrap();
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
                .status,
            "interrupted"
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
        assert!(
            !manager
                .inner
                .update(task["taskId"].as_str().unwrap(), "running", 1, None)
        );
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
                .is_some_and(|t| terminal(t.status))
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
                .is_some_and(|t| terminal(t.status))
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
}
