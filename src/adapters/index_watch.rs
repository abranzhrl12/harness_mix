//! Filesystem-driven incremental index maintenance. No LLM requests.
mod events;
mod pending;
mod report;
mod worker;

#[cfg(test)]
#[path = "index_watch/semantic_policy_events_tests.rs"]
mod semantic_policy_events;
#[cfg(test)]
#[path = "index_watch/tests.rs"]
mod tests;

use super::{edit_commit, project::Project};
use harness_domain::{Failure, execution::OperationContext};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::{Value, json};
use std::{
    fs::File,
    path::Path,
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
};

use pending::{MAX_PENDING, MAX_WAIT, Pending, QUIET, RECONCILE};

pub(crate) type Status = Arc<Mutex<Value>>;

/// Dropping the owner cancels the operation, wakes and joins the sole worker.
pub(crate) struct IndexWatch {
    queue: Arc<(Mutex<Pending>, Condvar)>,
    cancel: OperationContext,
    worker: Option<JoinHandle<()>>,
    pub(crate) status: Status,
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
        let pins = edit_commit::pin_directory(&project, &directory)?;
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
                        Ok(event) => events::relevant(&callback_root, event, &mut pending),
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
                // Keep the watcher, exclusive writer lease and ancestor pins alive
                // until the worker exits, including while a sync is cancelled.
                let (_watcher, _lease, _pins) = (watcher, lease, pins);
                worker::run(root, worker_queue, worker_status, worker_cancel);
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
