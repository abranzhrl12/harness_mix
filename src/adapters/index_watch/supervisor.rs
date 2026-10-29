use super::{IndexWatch, Status};
use harness_domain::Failure;
use serde_json::json;
use std::{
    path::Path,
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(crate) struct SupervisedWatch {
    pub(crate) status: Status,
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl SupervisedWatch {
    pub(crate) fn start(root: &Path) -> Result<Self, Failure> {
        Self::start_with_status(root, Arc::new(Mutex::new(json!({}))))
    }

    pub(crate) fn start_with_status(root: &Path, status: Status) -> Result<Self, Failure> {
        let initial = match IndexWatch::start(root) {
            Ok(watch) => Some(watch),
            Err(("SYNC_WATCH_BUSY", _)) => None,
            Err(error) => return Err(error),
        };
        *status
            .lock()
            .map_err(|_| ("INTERNAL", "Index watch status unavailable"))? = json!({
            "enabled": true, "role": if initial.is_some() { "owner" } else { "standby" },
            "state": "starting", "globalFreshnessCertified": false
        });
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let thread_stop = stop.clone();
        let thread_status = status.clone();
        let root = root.to_path_buf();
        let thread = std::thread::Builder::new()
            .name("harness-index-watch-supervisor".into())
            .spawn(move || {
                let mut active = initial;
                let mut started = epoch_seconds();
                loop {
                    if active.as_ref().is_some_and(|w| w.worker.as_ref().is_some_and(|t| t.is_finished())) {
                        active = None;
                    }
                    if active.is_none() {
                        match IndexWatch::start(&root) {
                            Ok(watch) => { active = Some(watch); started = epoch_seconds(); }
                            Err(("SYNC_WATCH_BUSY", _)) => {
                                let observed = super::metadata::observe(&root);
                                *thread_status.lock().unwrap_or_else(|e| e.into_inner()) = json!({
                                    "enabled":true, "role":"standby", "state":"waiting_for_owner",
                                    "reason":"Watcher lock held; owner telemetry is advisory and health is not verified",
                                    "observedOwner":observed,
                                    "globalFreshnessCertified":false
                                });
                            }
                            Err((code, message)) => {
                                *thread_status.lock().unwrap_or_else(|e| e.into_inner()) = json!({
                                    "enabled":true, "role":"standby", "state":"retrying",
                                    "lastError":code, "reason":message, "globalFreshnessCertified":false
                                });
                            }
                        }
                    }
                    if let Some(watch) = active.as_ref() {
                        let mut current = watch.status.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        current["role"] = json!("owner");
                        current["ownerPid"] = json!(std::process::id());
                        current["ownerStartedAtUnixSeconds"] = json!(started);
                        current["telemetryPublished"] = json!(super::metadata::publish(&root, &current, epoch_seconds()).is_ok());
                        *thread_status.lock().unwrap_or_else(|e| e.into_inner()) = current;
                    }
                    let (mutex, wake) = &*thread_stop;
                    let stopped = mutex.lock().unwrap_or_else(|e| e.into_inner());
                    if *stopped { break; }
                    let (stopped, _) = wake.wait_timeout(stopped, Duration::from_secs(2))
                        .unwrap_or_else(|e| e.into_inner());
                    if *stopped { break; }
                }
                drop(active);
            })
            .map_err(|_| ("SYNC_WATCH", "Cannot start index watch supervisor"))?;
        Ok(Self {
            status,
            stop,
            thread: Some(thread),
        })
    }
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Drop for SupervisedWatch {
    fn drop(&mut self) {
        let (mutex, wake) = &*self.stop;
        *mutex.lock().unwrap_or_else(|e| e.into_inner()) = true;
        wake.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
