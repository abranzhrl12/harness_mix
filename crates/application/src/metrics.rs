//! Bounded counters only: never retain request arguments, paths, session IDs or error text.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Default)]
struct Counters {
    attempted: u64,
    admitted: u64,
    rejected: u64,
    completed: u64,
    abandoned: u64,
    active: u64,
    peak_active: u64,
    ok: u64,
    error: u64,
    cancelled: u64,
    timeout: u64,
    queue_full: u64,
    rejected_cancelled: u64,
    rejected_timeout: u64,
    total_micros: u64,
    max_micros: u64,
}
impl Counters {
    fn snapshot(&self) -> Value {
        json!({"attempted":self.attempted,"admitted":self.admitted,"rejected":self.rejected,
            "completed":self.completed,"abandoned":self.abandoned,"active":self.active,"peakActive":self.peak_active,
            "outcomes":{"ok":self.ok,"error":self.error,"cancelled":self.cancelled,"timeout":self.timeout},
            "rejections":{"queueFull":self.queue_full,"cancelled":self.rejected_cancelled,"timeout":self.rejected_timeout},
            "admittedTerminalDurationMicros":{"total":self.total_micros,"max":self.max_micros}})
    }
}
#[derive(Default)]
struct State {
    total: Counters,
    tools: BTreeMap<&'static str, Counters>,
}
pub(crate) struct Metrics {
    pub(super) logs: crate::logging::Log,
    started: Instant,
    state: Mutex<State>,
}
impl Default for Metrics {
    fn default() -> Self {
        Self {
            logs: crate::logging::Log::default(),
            started: Instant::now(),
            state: Mutex::default(),
        }
    }
}
impl Metrics {
    pub fn with_logs(logs: crate::logging::Log) -> Self {
        Self {
            logs,
            ..Self::default()
        }
    }
    fn update(&self, name: &'static str, mut update: impl FnMut(&mut Counters)) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        update(&mut state.total);
        update(state.tools.entry(name).or_default());
    }
    #[cfg(test)]
    pub fn rejected(&self, name: &'static str, code: &str) {
        self.rejected_with_id(name, code, &harness_domain::execution::new_request_id());
    }
    pub fn rejected_with_id(&self, name: &'static str, code: &str, request_id: &str) {
        self.update(name, |counter| {
            counter.attempted = counter.attempted.saturating_add(1);
            counter.rejected = counter.rejected.saturating_add(1);
            match code {
                "QUEUE_FULL" => counter.queue_full = counter.queue_full.saturating_add(1),
                "CANCELLED" => {
                    counter.rejected_cancelled = counter.rejected_cancelled.saturating_add(1)
                }
                "TIMEOUT" => counter.rejected_timeout = counter.rejected_timeout.saturating_add(1),
                _ => {}
            }
        });
        self.logs.record_correlated(
            name,
            "rejected",
            Some("error"),
            Some(code),
            None,
            request_id,
        );
    }
    #[cfg(test)]
    pub fn admitted(self: &Arc<Self>, name: &'static str) -> Measurement {
        self.admitted_with_id(name, &harness_domain::execution::new_request_id())
    }
    pub fn admitted_with_id(self: &Arc<Self>, name: &'static str, request_id: &str) -> Measurement {
        self.update(name, |counter| {
            counter.attempted = counter.attempted.saturating_add(1);
            counter.admitted = counter.admitted.saturating_add(1);
            counter.active = counter.active.saturating_add(1);
            counter.peak_active = counter.peak_active.max(counter.active);
        });
        Measurement {
            request_id: request_id.to_owned(),
            metrics: self.clone(),
            name,
            started: Instant::now(),
            finished: false,
        }
    }
    pub fn snapshot(&self) -> Value {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let tools: serde_json::Map<String, Value> = state
            .tools
            .iter()
            .map(|(name, counters)| (name.to_string(), counters.snapshot()))
            .collect();
        json!({"available":true,"scope":"application_instance","persistent":false,"sessionSpecific":false,
            "measurement":"application operation outcomes, not RPC delivery or billing",
            "currentQueryIncludedAsActive":true,"uptimeMs":self.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            "totals":state.total.snapshot(),"byTool":tools})
    }
}
pub(crate) struct Measurement {
    request_id: String,
    metrics: Arc<Metrics>,
    name: &'static str,
    started: Instant,
    finished: bool,
}
impl Measurement {
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
    fn terminal(&mut self, outcome: Option<&str>, code: Option<&str>) {
        let elapsed = self.started.elapsed().as_micros().min(u64::MAX as u128) as u64;
        self.metrics.update(self.name, |counter| {
            counter.active = counter.active.saturating_sub(1);
            counter.total_micros = counter.total_micros.saturating_add(elapsed);
            counter.max_micros = counter.max_micros.max(elapsed);
            if let Some(outcome) = outcome {
                counter.completed = counter.completed.saturating_add(1);
                let selected = match outcome {
                    "ok" => &mut counter.ok,
                    "cancelled" => &mut counter.cancelled,
                    "timeout" => &mut counter.timeout,
                    _ => &mut counter.error,
                };
                *selected = selected.saturating_add(1);
            } else {
                counter.abandoned = counter.abandoned.saturating_add(1);
            }
        });
        self.metrics.logs.record_correlated(
            self.name,
            if outcome.is_some() {
                "completed"
            } else {
                "abandoned"
            },
            outcome,
            code,
            Some(elapsed),
            &self.request_id,
        );
        self.finished = true;
    }
    #[cfg(test)]
    pub fn finish(&mut self, outcome: &str) {
        if !self.finished {
            self.terminal(Some(outcome), None);
        }
    }
    pub fn finish_envelope(&mut self, envelope: &Value) {
        if !self.finished {
            self.terminal(
                Some(envelope["state"].as_str().unwrap_or("error")),
                envelope["diagnostics"][0]["code"].as_str(),
            );
        }
    }
}
impl Drop for Measurement {
    fn drop(&mut self) {
        if !self.finished {
            self.terminal(None, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejection_and_raii_terminal_accounting_are_distinct_and_idempotent() {
        let metrics = Arc::new(Metrics::default());
        metrics.rejected("harness_read", "QUEUE_FULL");
        {
            let mut guard = metrics.admitted("harness_read");
            guard.finish("ok");
            guard.finish("error");
        }
        drop(metrics.admitted("harness_read"));
        let snapshot = metrics.snapshot();
        let total = &snapshot["totals"];
        assert_eq!(total["attempted"], 3);
        assert_eq!(total["admitted"], 2);
        assert_eq!(total["rejected"], 1);
        assert_eq!(total["completed"], 1);
        assert_eq!(total["abandoned"], 1);
        assert_eq!(total["active"], 0);
        assert_eq!(total["outcomes"]["ok"], 1);
        assert_eq!(total["outcomes"]["error"], 0);
        assert_eq!(total["rejections"]["queueFull"], 1);
        assert_eq!(snapshot["byTool"]["harness_read"], *total);
    }

    #[test]
    fn outcome_buckets_are_distinct() {
        let metrics = Arc::new(Metrics::default());
        for state in ["ok", "error", "cancelled", "timeout"] {
            metrics.admitted("harness_verify").finish(state);
        }
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot["totals"]["completed"], 4);
        assert_eq!(snapshot["totals"]["active"], 0);
        for state in ["ok", "error", "cancelled", "timeout"] {
            assert_eq!(snapshot["totals"]["outcomes"][state], 1);
        }
    }

    #[test]
    fn concurrent_guards_preserve_aggregate_counts() {
        let metrics = Arc::new(Metrics::default());
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let metrics = metrics.clone();
                std::thread::spawn(move || {
                    for index in 0..100 {
                        let mut guard = metrics.admitted("harness_read");
                        if index % 2 == 0 {
                            guard.finish("ok");
                        }
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot["totals"]["attempted"], 800);
        assert_eq!(snapshot["totals"]["completed"], 400);
        assert_eq!(snapshot["totals"]["abandoned"], 400);
        assert_eq!(snapshot["totals"]["active"], 0);
    }
}
