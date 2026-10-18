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
                let mut backoff = Duration::from_secs(5);
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
                            }
                            let due = if now >= next_full {
                                next_full
                            } else {
                                pending.ready_at().unwrap_or(next_full).min(next_full)
                            }
                            .max(retry_at);
                            if now >= due {
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
                    let successful = result.as_ref().is_ok_and(|r| {
                        r["selectedScopeApplied"] == true
                            && (!full
                                || (r["discoveryComplete"] == true
                                    && r["uncheckedFiles"] == 0
                                    && r["unsupportedChangedFiles"] == 0
                                    && r["restrictedIndexedFiles"] == 0))
                    });
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
                    value["lastError"] = if successful {
                        Value::Null
                    } else {
                        json!(
                            result
                                .as_ref()
                                .err()
                                .map(|e| e.0)
                                .or_else(|| result
                                    .as_ref()
                                    .ok()
                                    .and_then(|r| r["failure"]["code"].as_str()))
                                .unwrap_or("SYNC_INCOMPLETE")
                        )
                    };
                    value["state"] = json!(if successful { "idle" } else { "retrying" });
                    drop(value);
                    if successful {
                        backoff = Duration::from_secs(5);
                        retry_at = Instant::now();
                    } else {
                        retry_at = Instant::now() + backoff;
                        backoff = (backoff * 2).min(Duration::from_secs(60));
                        let mut pending = mutex.lock().unwrap_or_else(|e| e.into_inner());
                        if full {
                            pending.reconcile();
                        } else {
                            for path in paths {
                                pending.add(path);
                            }
                        }
                        retry_revision = pending.revision;
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
