use super::{
    Status,
    pending::{Pending, RECONCILE},
    report::{retry_delay, sync_report},
};
use crate::adapters::index_store;
use harness_domain::execution::OperationContext;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

pub(super) fn run(
    root: PathBuf,
    queue: Arc<(Mutex<Pending>, Condvar)>,
    status: Status,
    cancel: OperationContext,
) {
    let (mutex, wake) = &*queue;
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
            let mut value = status.lock().unwrap_or_else(|e| e.into_inner());
            value["state"] = json!("syncing");
        }
        let started = Instant::now();
        let context = cancel.with_execution_timeout(Some(Duration::from_secs(60)));
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
        let mut value = status.lock().unwrap_or_else(|e| e.into_inner());
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
        if successful {
            value["lastSuccessfulSyncUnixSeconds"] = json!(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
        }
        value["lastError"] = report["lastError"].clone();
        value["lastResult"] = report.clone();
        if full {
            value["lastReconciliation"] = report;
        }
        value["consecutiveFailures"] = json!(consecutive_failures);
        value["retryScheduled"] = json!(retry.is_some());
        value["retryDelayMs"] = retry.map(|d| json!(d.as_millis())).unwrap_or(Value::Null);
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
}
