//! Filesystem-driven incremental index maintenance. No LLM requests.
use super::{index_extract, index_inventory, index_store, project::Project};
use harness_domain::{Failure, execution::OperationContext};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::File,
    path::Path,
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

const MAX_PENDING: usize = 1024;
const QUIET: Duration = Duration::from_millis(600);
const MAX_WAIT: Duration = Duration::from_secs(2);
const RECONCILE: Duration = Duration::from_secs(300);
pub(crate) type Status = Arc<Mutex<Value>>;

#[derive(Default)]
struct Pending {
    paths: BTreeSet<String>,
    full: bool,
    first: Option<Instant>,
    last: Option<Instant>,
    stop: bool,
    revision: u64,
}
impl Pending {
    fn dirty(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        let now = Instant::now();
        self.first.get_or_insert(now);
        self.last = Some(now);
    }
    fn reconcile(&mut self) {
        self.full = true;
        self.paths.clear();
        self.dirty();
    }
    fn add(&mut self, path: String) {
        if !self.full {
            if self.paths.len() >= MAX_PENDING && !self.paths.contains(&path) {
                self.reconcile();
            } else {
                self.paths.insert(path);
            }
        }
        self.dirty();
    }
    fn ready_at(&self) -> Option<Instant> {
        Some((self.last? + QUIET).min(self.first? + MAX_WAIT))
    }
}

/// Dropping the owner cancels the operation, wakes and joins the sole worker.
pub(crate) struct IndexWatch {
    queue: Arc<(Mutex<Pending>, Condvar)>,
    cancel: OperationContext,
    worker: Option<JoinHandle<()>>,
    pub(crate) status: Status,
}

fn relevant(root: &Path, event: Event, pending: &mut Pending) {
    if event.need_rescan() {
        pending.reconcile();
        return;
    }
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    if event.paths.is_empty() {
        pending.reconcile();
    }
    for path in event.paths {
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let Some(relative) = relative.to_str() else {
            pending.reconcile();
            continue;
        };
        let relative = relative.replace('\\', "/");
        if relative.len() > 1000 {
            pending.reconcile();
            continue;
        }
        if relative.split('/').any(index_inventory::excluded)
            || relative == ".agents"
            || relative.starts_with(".agents/")
            || index_inventory::restricted(&relative)
        {
            continue;
        }
        if relative.is_empty()
            || relative.ends_with(".gitignore")
            || relative.ends_with(".json")
            || matches!(event.kind,
                EventKind::Create(notify::event::CreateKind::Folder)
                | EventKind::Remove(notify::event::RemoveKind::Folder))
            // Windows directory rename notifications may have an unknown type.
            || path.extension().is_none()
        {
            pending.reconcile();
        } else if index_extract::language(&relative).is_some() {
            pending.add(relative);
        }
    }
}

// Keep a bounded diagnostic report instead of discarding the reason a full
// reconciliation was incomplete. File bodies and per-file reports stay out.
fn sync_report(result: &Result<Value, Failure>, full: bool) -> Value {
    let mut report = json!({"complete":false,"retryable":false,"failure":null});
    match result {
        Ok(value) => {
            for key in [
                "selectedScopeApplied",
                "discoveryComplete",
                "uncheckedFiles",
                "unsupportedChangedFiles",
                "restrictedIndexedFiles",
                "restrictedIndexedReasons",
                "nativeFilesRemaining",
                "filesProcessed",
                "changedFilesCommitted",
            ] {
                if let Some(value) = value.get(key) {
                    report[key] = value.clone();
                }
            }
            if let Some(failure) = value.get("failure").filter(|v| !v.is_null()) {
                report["failure"] = failure.clone();
            }
            report["complete"] = json!(
                value["selectedScopeApplied"] == true
                    && report["failure"].is_null()
                    && (!full
                        || (value["discoveryComplete"] == true
                            && value["uncheckedFiles"] == 0
                            && value["unsupportedChangedFiles"] == 0))
            );
        }
        Err((code, message)) => {
            report["failure"] = json!({"code":code,"message":message});
        }
    }
    if report["complete"] != true {
        let code = report["failure"]["code"]
            .as_str()
            .unwrap_or("SYNC_INCOMPLETE");
        // Coverage gaps and deterministic incompatibilities require a change,
        // not another identical full inventory after a short backoff.
        let retryable = !matches!(
            code,
            "SYNC_INCOMPLETE"
                | "SYNC_LANGUAGE"
                | "SYNC_UNSUPPORTED_RELATION"
                | "SYNC_LEGACY_REFERENCE"
                | "SYNC_SCHEMA"
                | "SYNC_CONFIG"
                | "SYNC_LIMIT"
                | "SYNC_INTEGRITY"
                | "SYNC_COMMIT_UNCERTAIN"
                | "ENCODING"
                | "PATH"
                | "LIMIT"
                | "CANCELLED"
        );
        report["lastError"] = json!(code);
        report["retryable"] = json!(retryable);
    } else {
        report["lastError"] = Value::Null;
    }
    report
}

// At most five attempts between source events/periodic reconciliations.
fn retry_delay(report: &Value, failures: u32) -> Option<Duration> {
    (report["retryable"] == true && (1..5).contains(&failures))
        .then(|| Duration::from_secs(5_u64 << (failures - 1)))
}

impl IndexWatch {
    pub(crate) fn start(root: &Path) -> Result<Self, Failure> {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
        };
        let project = Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?;
        // Existing index only; enabling maintenance never initializes/migrates it.
        project.target(".codegraph/codegraph.db")?;
        let directory = project.target(".codegraph")?;
        let pins = super::edit_commit::pin_directory(&project, &directory)?;
        let lock = project.target_optional(".codegraph/autosync.lock")?;
        let lease = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .custom_flags(0x0020_0000)
            .open(lock)
            .map_err(|_| {
                (
                    "SYNC_WATCH_BUSY",
                    "Another automatic index writer owns this project, or its lock is unavailable",
                )
            })?;
        let mut identity = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(lease.as_raw_handle()), &mut identity) }
            .map_err(|_| ("PATH", "Cannot inspect index watcher lock"))?;
        if identity.nNumberOfLinks != 1
            || identity.dwFileAttributes & 0x400 != 0
            || lease
                .metadata()
                .map_err(|_| ("PATH", "Cannot inspect lock"))?
                .len()
                != 0
        {
            return Err(("PATH", "Unsafe index watcher lock"));
        }
        let root = project.root;
        let queue = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let callback_queue = queue.clone();
        let callback_root = root.clone();
        let mut watcher = RecommendedWatcher::new(
            move |event: notify::Result<Event>| {
                let (mutex, wake) = &*callback_queue;
                if let Ok(mut pending) = mutex.lock() {
                    match event {
                        Ok(event) => relevant(&callback_root, event, &mut pending),
                        Err(_) => pending.reconcile(),
                    }
                    if pending.first.is_some() {
                        wake.notify_one();
                    }
                }
            },
            Config::default().with_follow_symlinks(false),
        )
        .map_err(|_| ("SYNC_WATCH", "Filesystem notifications unavailable"))?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|_| ("SYNC_WATCH", "Cannot watch project directory"))?;
        let status = Arc::new(Mutex::new(json!({"enabled":true,"state":"starting",
            "backend":"native_filesystem_events","debounceMs":QUIET.as_millis(),
            "maxDelayMs":MAX_WAIT.as_millis(),"reconcileSeconds":RECONCILE.as_secs(),
            "maxPendingPaths":MAX_PENDING,"batches":0,"reconciliations":0,
            "changedFilesCommitted":0,"lastError":null,"globalFreshnessCertified":false})));
        let worker_queue = queue.clone();
        let worker_status = status.clone();
        let cancel = OperationContext::without_deadline();
        let worker_cancel = cancel.clone();
        let worker = std::thread::Builder::new()
            .name("harness-index-sync".into())
            .spawn(move || {
                let (_watcher, _lease, _pins) = (watcher, lease, pins);
                let (mutex, wake) = &*worker_queue;
                let mut next_full = Instant::now();
                let mut retry_at = Instant::now();
                let mut consecutive_failures = 0_u32;
                let mut retry_revision = 0;
                loop {
                    let (full, paths) = {
                        let mut pending = mutex.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if pending.stop {
                                return;
                            }
                            let now = Instant::now();
                            // A new save bypasses the backoff of an older error.
                            if pending.revision != retry_revision {
                                retry_at = now;
                                consecutive_failures = 0;
                                retry_revision = pending.revision;
                            }
                            let due = if now >= next_full {
                                next_full
                            } else {
                                pending.ready_at().unwrap_or(next_full).min(next_full)
                            }
                            .max(retry_at);
                            if now >= due {
                                if now >= next_full && !pending.full {
                                    consecutive_failures = 0;
                                }
                                let full = pending.full || now >= next_full;
                                let paths = std::mem::take(&mut pending.paths);
                                pending.full = false;
                                pending.first = None;
                                pending.last = None;
                                break (full, paths);
                            }
                            pending = wake
                                .wait_timeout(pending, due.saturating_duration_since(now))
                                .unwrap_or_else(|e| e.into_inner())
                                .0;
                        }
                    };
                    {
                        let mut value = worker_status.lock().unwrap_or_else(|e| e.into_inner());
                        value["state"] = json!("syncing");
                    }
                    let started = Instant::now();
                    let context =
                        worker_cancel.with_execution_timeout(Some(Duration::from_secs(60)));
                    let result = if full {
                        index_store::sync_workspace_all(&root, &context)
                    } else {
                        index_store::sync_changed_paths(&root, &paths, &context)
                    };
                    // Worktree fallback may run a full scan inside a path batch.
                    let full = full
                        || result
                            .as_ref()
                            .is_ok_and(|r| r.get("discoveryComplete").is_some());
                    let report = sync_report(&result, full);
                    let successful = report["complete"] == true;
                    consecutive_failures = if successful {
                        0
                    } else {
                        consecutive_failures.saturating_add(1)
                    };
                    let retry = retry_delay(&report, consecutive_failures);
                    if full {
                        next_full = Instant::now() + RECONCILE;
                    }
                    let mut value = worker_status.lock().unwrap_or_else(|e| e.into_inner());
                    let key = if full { "reconciliations" } else { "batches" };
                    value[key] = json!(value[key].as_u64().unwrap_or(0) + 1);
                    value["lastDurationMs"] = json!(started.elapsed().as_millis());
                    value["lastBatchPaths"] = json!(paths.len());
                    let changed = result
                        .as_ref()
                        .ok()
                        .and_then(|r| r["changedFilesCommitted"].as_u64())
                        .unwrap_or(0);
                    value["changedFilesCommitted"] =
                        json!(value["changedFilesCommitted"].as_u64().unwrap_or(0) + changed);
                    value["lastError"] = report["lastError"].clone();
                    value["lastResult"] = report.clone();
                    if full {
                        value["lastReconciliation"] = report;
                    }
                    value["consecutiveFailures"] = json!(consecutive_failures);
                    value["retryScheduled"] = json!(retry.is_some());
                    value["retryDelayMs"] =
                        retry.map(|d| json!(d.as_millis())).unwrap_or(Value::Null);
                    // A healthy incremental batch must not erase a coverage gap.
                    let incomplete = value["lastReconciliation"]["complete"] == false;
                    value["state"] = json!(if retry.is_some() {
                        "retrying"
                    } else if !successful || incomplete {
                        "degraded"
                    } else {
                        "idle"
                    });
                    drop(value);
                    if let Some(delay) = retry {
                        retry_at = Instant::now() + delay;
                        let mut pending = mutex.lock().unwrap_or_else(|e| e.into_inner());
                        let new_events = pending.revision != retry_revision;
                        if full {
                            pending.reconcile();
                        } else {
                            for path in paths {
                                pending.add(path);
                            }
                        }
                        // Do not absorb a save received while sync was running
                        // into the synthetic retry's revision/backoff.
                        if !new_events {
                            retry_revision = pending.revision;
                        }
                    } else {
                        // Keep notifications received during this operation. No
                        // synthetic retry: wait for an event or periodic scan.
                        retry_at = Instant::now();
                    }
                }
            })
            .map_err(|_| ("SYNC_WATCH", "Cannot start index worker"))?;
        Ok(Self {
            queue,
            cancel,
            worker: Some(worker),
            status,
        })
    }
}

impl Drop for IndexWatch {
    fn drop(&mut self) {
        self.cancel.cancel();
        let (mutex, wake) = &*self.queue;
        mutex.lock().unwrap_or_else(|e| e.into_inner()).stop = true;
        wake.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait(mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !predicate() {
            assert!(Instant::now() < deadline, "watcher did not converge");
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    fn count(root: &Path, query: &str) -> i64 {
        rusqlite::Connection::open_with_flags(
            root.join(".codegraph/codegraph.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .ok()
        .and_then(|db| db.query_row(query, [], |r| r.get(0)).ok())
        .unwrap_or(-1)
    }

    #[test]
    fn native_watcher_updates_only_changed_files_handles_rename_delete_and_releases_lease() {
        let root = tempfile::tempdir().unwrap();
        index_store::initialize(root.path(), &OperationContext::default()).unwrap();
        std::fs::write(root.path().join("a.rs"), "pub fn before() {}\n").unwrap();
        std::fs::write(root.path().join("b.rs"), "pub fn untouched() {}\n").unwrap();
        std::fs::write(root.path().join(".gitignore"), "ignored.rs\n").unwrap();
        let watch = IndexWatch::start(root.path()).unwrap();
        assert!(
            IndexWatch::start(root.path()).is_err(),
            "one automatic writer per project"
        );
        wait(|| watch.status.lock().unwrap()["state"] == "idle");
        assert_eq!(count(root.path(), "SELECT count(*) FROM files"), 2);
        let untouched = count(
            root.path(),
            "SELECT indexed_at FROM files WHERE path='b.rs'",
        );
        for _ in 0..8 {
            std::fs::write(root.path().join("a.rs"), "pub fn after() {}\n").unwrap();
        }
        wait(|| count(root.path(), "SELECT count(*) FROM nodes WHERE name='after'") == 1);
        assert_eq!(
            count(
                root.path(),
                "SELECT indexed_at FROM files WHERE path='b.rs'"
            ),
            untouched
        );
        assert_eq!(watch.status.lock().unwrap()["reconciliations"], 1);
        std::fs::write(root.path().join("a.rs"), "fn incomplete( {").unwrap();
        wait(|| watch.status.lock().unwrap()["state"] == "retrying");
        {
            let status = watch.status.lock().unwrap();
            assert_eq!(status["lastResult"]["failure"]["path"], "a.rs");
            assert!(status["lastResult"]["failure"]["message"].is_string());
            assert_eq!(status["retryScheduled"], true);
        }
        // Keep the last confirmed graph while the editor holds incomplete syntax.
        assert_eq!(
            count(root.path(), "SELECT count(*) FROM nodes WHERE name='after'"),
            1
        );
        std::fs::write(root.path().join("a.rs"), "pub fn recovered() {}\n").unwrap();
        wait(|| {
            count(
                root.path(),
                "SELECT count(*) FROM nodes WHERE name='recovered'",
            ) == 1
        });
        assert_eq!(watch.status.lock().unwrap()["reconciliations"], 1);
        std::fs::rename(root.path().join("a.rs"), root.path().join("renamed.rs")).unwrap();
        wait(|| {
            count(
                root.path(),
                "SELECT count(*) FROM files WHERE path='renamed.rs'",
            ) == 1
                && count(root.path(), "SELECT count(*) FROM files WHERE path='a.rs'") == 0
        });
        std::fs::remove_file(root.path().join("renamed.rs")).unwrap();
        wait(|| {
            count(
                root.path(),
                "SELECT count(*) FROM files WHERE path='renamed.rs'",
            ) == 0
        });
        let batches = watch.status.lock().unwrap()["batches"].as_u64().unwrap();
        std::fs::write(root.path().join("ignored.rs"), "pub fn ignored() {}\n").unwrap();
        wait(|| watch.status.lock().unwrap()["batches"].as_u64().unwrap() > batches);
        assert_eq!(
            count(
                root.path(),
                "SELECT count(*) FROM files WHERE path='ignored.rs'"
            ),
            0
        );
        drop(watch);
        let restarted = IndexWatch::start(root.path()).unwrap();
        wait(|| restarted.status.lock().unwrap()["state"] == "idle");
    }

    #[test]
    fn incomplete_coverage_waits_for_events_and_retains_its_diagnosis() {
        let root = tempfile::tempdir().unwrap();
        index_store::initialize(root.path(), &OperationContext::default()).unwrap();
        std::fs::write(root.path().join("a.rs"), "pub fn before() {}\n").unwrap();
        std::fs::write(root.path().join("large.rs"), vec![b' '; 1024 * 1024 + 1]).unwrap();
        let watch = IndexWatch::start(root.path()).unwrap();
        wait(|| watch.status.lock().unwrap()["state"] == "degraded");
        {
            let status = watch.status.lock().unwrap();
            assert_eq!(status["lastError"], "SYNC_INCOMPLETE");
            assert_eq!(status["lastReconciliation"]["uncheckedFiles"], 1);
            assert_eq!(status["retryScheduled"], false);
            assert!(status["retryDelayMs"].is_null());
        }
        // Longer than the old initial backoff: no repeated whole-workspace scan.
        std::thread::sleep(Duration::from_secs(6));
        assert_eq!(watch.status.lock().unwrap()["reconciliations"], 1);
        std::fs::write(root.path().join("a.rs"), "pub fn after() {}\n").unwrap();
        wait(|| count(root.path(), "SELECT count(*) FROM nodes WHERE name='after'") == 1);
        wait(|| watch.status.lock().unwrap()["batches"].as_u64().unwrap() > 0);
        {
            let status = watch.status.lock().unwrap();
            assert_eq!(status["state"], "degraded");
            assert_eq!(status["lastReconciliation"]["uncheckedFiles"], 1);
            assert_eq!(status["reconciliations"], 1);
        }
        // Removing the coverage obstacle and reconciling clears the diagnosis.
        std::fs::remove_file(root.path().join("large.rs")).unwrap();
        {
            let (mutex, wake) = &*watch.queue;
            mutex.lock().unwrap().reconcile();
            wake.notify_one();
        }
        wait(|| watch.status.lock().unwrap()["state"] == "idle");
        assert_eq!(
            watch.status.lock().unwrap()["lastReconciliation"]["complete"],
            true
        );
    }

    #[test]
    fn retry_policy_is_bounded_and_preserves_failure_details() {
        for code in [
            "SYNC_UNSUPPORTED_RELATION",
            "SYNC_SCHEMA",
            "SYNC_LIMIT",
            "SYNC_CONFIG",
            "SYNC_COMMIT_UNCERTAIN",
        ] {
            let report = sync_report(&Err((code, "Needs a change")), true);
            assert_eq!(report["lastError"], code);
            assert_eq!(report["failure"]["message"], "Needs a change");
            assert!(retry_delay(&report, 1).is_none());
        }
        let report = sync_report(&Err(("STALE", "Source changed")), false);
        for (failures, seconds) in [(1, 5), (2, 10), (3, 20), (4, 40)] {
            assert_eq!(
                retry_delay(&report, failures),
                Some(Duration::from_secs(seconds))
            );
        }
        assert!(retry_delay(&report, 0).is_none());
        assert!(retry_delay(&report, 5).is_none());
        assert!(retry_delay(&report, u32::MAX).is_none());
        let report = sync_report(
            &Ok(json!({
                "selectedScopeApplied":false,
                "failure":{"path":"src/a.rs","code":"STALE","message":"Source changed","commitState":"not_committed"},
                "changedFilesCommitted":2
            })),
            false,
        );
        assert_eq!(report["failure"]["path"], "src/a.rs");
        assert_eq!(report["changedFilesCommitted"], 2);
        assert!(retry_delay(&report, 1).is_some());
    }

    #[test]
    fn coverage_limitations_are_not_transient_failures() {
        let clean = json!({"selectedScopeApplied":true,"discoveryComplete":true,
            "uncheckedFiles":0,"unsupportedChangedFiles":0,"restrictedIndexedFiles":0});
        assert_eq!(sync_report(&Ok(clean.clone()), true)["complete"], true);
        for (key, value) in [
            ("discoveryComplete", json!(false)),
            ("uncheckedFiles", json!(1)),
            ("unsupportedChangedFiles", json!(1)),
        ] {
            let mut incomplete = clean.clone();
            incomplete[key] = value;
            let report = sync_report(&Ok(incomplete), true);
            assert_eq!(report["complete"], false);
            assert_eq!(report["lastError"], "SYNC_INCOMPLETE");
            assert!(retry_delay(&report, 1).is_none());
        }
        assert_eq!(
            sync_report(&Ok(json!({"selectedScopeApplied":true})), false)["complete"],
            true
        );
        let mut excluded = clean;
        excluded["restrictedIndexedFiles"] = json!(3);
        excluded["restrictedIndexedReasons"] = json!({"config_filename":3});
        let report = sync_report(&Ok(excluded), true);
        assert_eq!(report["complete"], true);
        assert_eq!(report["restrictedIndexedReasons"]["config_filename"], 3);
        assert!(report["lastError"].is_null());
        assert!(retry_delay(&report, 1).is_none());
    }

    #[test]
    fn configuration_events_and_rescan_flags_schedule_reconciliation() {
        let root = Path::new("C:/project");
        for name in ["codegraph.json", "tsconfig.json", "src/.gitignore"] {
            let mut pending = Pending::default();
            relevant(
                root,
                Event::new(EventKind::Any).add_path(root.join(name)),
                &mut pending,
            );
            assert!(pending.full);
        }
        let mut pending = Pending::default();
        let mut event = Event::new(EventKind::Other);
        event.attrs.set_flag(notify::event::Flag::Rescan);
        relevant(root, event, &mut pending);
        assert!(pending.full);
    }
    #[test]
    fn bursts_are_bounded_and_generated_events_do_not_schedule_work() {
        let root = Path::new("C:/project");
        let mut pending = Pending::default();
        for path in [
            "target/a.rs",
            ".codegraph/codegraph.db-wal",
            ".agents/log.rs",
            ".agents",
            ".env",
            "README.md",
        ] {
            relevant(
                root,
                Event::new(EventKind::Any).add_path(root.join(path)),
                &mut pending,
            );
        }
        assert!(pending.first.is_none());
        for _ in 0..100 {
            pending.add("src/a.rs".into());
        }
        assert_eq!(pending.paths.len(), 1);
        assert!(pending.ready_at().unwrap() <= pending.first.unwrap() + MAX_WAIT);
        for i in 0..MAX_PENDING + 1 {
            pending.add(format!("src/a{i}.rs"));
        }
        assert!(pending.full);
        assert!(pending.paths.is_empty());
    }
}
