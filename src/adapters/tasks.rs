//! Durable background verification with bounded concurrency and replayable events.
#[path = "observe.rs"]
mod observe;
use super::{process::NativeProcessRunner, tasks_store::Store, verify};
use harness_domain::{
    Failure,
    contracts::VerifyArgs,
    execution::OperationContext,
    jobs::{JobLifecycle, JobStatus},
    process::{ProcessOutcome, ProcessSpec},
    tasks::TaskArgs,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
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
        Self {
            inner: self.inner.clone(),
        }
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
    lifecycle: JobLifecycle,
    context: OperationContext,
    wake: Arc<Notify>,
    result: Option<Value>,
    configuration: Value,
}
impl Task {
    fn value(&self) -> Value {
        json!({"taskId":self.id,"status":self.lifecycle.status().as_str(),"attempt":self.lifecycle.attempt(),"configuration":self.configuration,"result":self.result})
    }
}
fn terminal(status: JobStatus) -> bool {
    status.is_terminal()
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
        let lifecycle = JobLifecycle::restore(
            value["status"].as_str().ok_or(invalid)?,
            attempt,
            u8::try_from(maximum).map_err(|_| invalid)?,
        )
        .ok_or(invalid)?;
        state.tasks.push_back(Task {
            id,
            lifecycle,
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
            if !terminal(task.lifecycle.status()) {
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
    fn update(&self, id: &str, status: JobStatus, attempt: u8, result: Option<Value>) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(task) = state.tasks.iter_mut().find(|t| t.id == id) {
            if task.lifecycle.transition(status, attempt).is_err() {
                return false;
            }
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
        json!({"status":if error.is_some(){"error"}else if state.store.is_some(){"ok"}else{"skipped"},"storage":if error.is_some(){"error"}else if state.store.is_some(){"ready"}else{"lazy"},"persistenceError":error.map(|(code,message)|json!({"code":code,"message":message})),"retained":state.tasks.len(),"active":state.tasks.iter().filter(|t|!terminal(t.lifecycle.status())).count(),"cursor":state.sequence,"schemaChecked":state.store.is_some()&&error.is_none(),"readChecked":matches!(read,Some(Ok(_)))&&error.is_none(),"writeabilityChecked":false})
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
            value["state"] = json!(task.lifecycle.status().as_str());
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
                        || !terminal(task.lifecycle.status()))
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
                    .filter(|t| !terminal(t.lifecycle.status()))
                    .map(|t| t.id.clone())
                    .collect();
                for id in ids {
                    let task = state.tasks.iter_mut().find(|t| t.id == id).unwrap();
                    if !task.lifecycle.interrupt_for_recovery() {
                        continue;
                    }
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
            if !terminal(task.lifecycle.status()) {
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
        })
        .await
        .is_ok()
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
        if args.action != "events" && args.action != "observe" && args.after.is_some() {
            return Err(("ARGUMENT", "after requires events or observe"));
        }
        if matches!(args.action.as_str(), "start" | "list") && args.task_id.is_some() {
            return Err(("ARGUMENT", "taskId is not valid for start or list"));
        }
        if args.action != "observe"
            && (args.filter.is_some()
                || args.lines.is_some()
                || args.before.is_some()
                || args.stream.is_some()
                || args.cursor.is_some()
                || args.max_chars.is_some())
        {
            return Err((
                "ARGUMENT",
                "filter, lines, before, stream, cursor and maxChars require observe",
            ));
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
                    json!({"tasks":state.tasks.iter().map(|t| json!({"taskId":t.id,"status":t.lifecycle.status().as_str(),"attempt":t.lifecycle.attempt()})).collect::<Vec<_>>(),"cursor":state.sequence,"concurrency":2,"persistenceError":persistence_error(&state)}),
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
                if args.action == "cancel" && !terminal(task.lifecycle.status()) {
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
            "observe" => {
                let observe_args = args.into_observe()?;
                self.observe(&observe_args)
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
            if state
                .tasks
                .iter()
                .filter(|t| !terminal(t.lifecycle.status()))
                .count()
                >= MAX_ACTIVE
            {
                return Err(("TASK_BUSY", "Background task queue is full"));
            }
            while state.tasks.len() >= MAX_RETAINED {
                if let Some(i) = state
                    .tasks
                    .iter()
                    .position(|t| terminal(t.lifecycle.status()))
                {
                    state.tasks.remove(i);
                } else {
                    break;
                }
            }
            let configuration = json!({"verify":verify_args,"timeoutSeconds":args.timeout_seconds,"maxAttempts":attempts});
            let task = Task {
                id: id.clone(),
                lifecycle: JobLifecycle::queued(),
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

#[path = "tasks/execute.rs"]
mod execute;
use execute::execute;

#[cfg(test)]
#[path = "tasks/tests.rs"]
mod tests;
