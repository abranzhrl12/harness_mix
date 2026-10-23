//! Aggregate optional diagnostic facts independently of the per-file preview cap.
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Totals {
    operations: BTreeMap<String, (u64, u64)>,
    counters: BTreeMap<String, u64>,
    paths: BTreeMap<String, u64>,
    omitted: u64,
}
impl Totals {
    pub(super) fn record(&mut self, result: &Value) {
        let metrics = &result["profileMetrics"];
        if let Some(operations) = metrics["operations"].as_object() {
            for (name, value) in operations {
                let entry = self.operations.entry(name.clone()).or_default();
                entry.0 += value["calls"].as_u64().unwrap_or(0);
                entry.1 += value["totalNanos"].as_u64().unwrap_or(0);
            }
        }
        if let Some(counters) = metrics["counters"].as_object() {
            for (name, value) in counters {
                *self.counters.entry(name.clone()).or_default() += value.as_u64().unwrap_or(0);
            }
        }
        if let Some(paths) = metrics["sourceReads"].as_object() {
            for (path, count) in paths {
                let count = count.as_u64().unwrap_or(0);
                if path.len() <= 1000 && (self.paths.contains_key(path) || self.paths.len() < 1024)
                {
                    *self.paths.entry(path.clone()).or_default() += count;
                } else {
                    self.omitted += count;
                }
            }
        }
    }
    pub(super) fn write(&self, report: &mut Value) {
        if self.operations.is_empty() && self.counters.is_empty() {
            return;
        }
        let operations: serde_json::Map<String, Value> = self
            .operations
            .iter()
            .map(|(name, (calls, nanos))| {
                (
                    name.clone(),
                    json!({"calls":calls,"totalMicros":*nanos as f64/1000.0,
                "meanMicros":if *calls==0 {0.0}else{*nanos as f64/1000.0/(*calls as f64)}}),
                )
            })
            .collect();
        let omitted = self.omitted
            + self
                .counters
                .get("source_paths_omitted")
                .copied()
                .unwrap_or(0);
        report["profileMetrics"] = json!({"operations":operations,"counters":self.counters,
            "sourceReads":self.paths,"sourcePathsOmittedAttempts":omitted,
            "uniqueSourcePathsRecorded":self.paths.len(),
            "coverage":"successful_publications_main_thread; preparation_workers_not_included; nested_wall_times_do_not_sum"});
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn totals_include_every_publication_and_compute_weighted_mean() {
        let mut total = Totals::default();
        for _ in 0..100 {
            total.record(
                &json!({"profileMetrics":{"operations":{"read":{"calls":2,"totalNanos":6000}},
                "counters":{"bytes":10},"sourceReads":{"a.rs":2}}}),
            );
        }
        let mut report = json!({});
        total.write(&mut report);
        assert_eq!(report["profileMetrics"]["operations"]["read"]["calls"], 200);
        assert_eq!(
            report["profileMetrics"]["operations"]["read"]["meanMicros"],
            3.0
        );
        assert_eq!(report["profileMetrics"]["sourceReads"]["a.rs"], 200);
        assert_eq!(report["profileMetrics"]["counters"]["bytes"], 1000);
    }
    #[test]
    fn missing_metrics_are_ignored_and_path_limits_report_omissions() {
        let mut total = Totals::default();
        total.record(&json!({}));
        let mut report = json!({});
        total.write(&mut report);
        assert!(report["profileMetrics"].is_null());
        for i in 0..1030 {
            total.record(
                &json!({"profileMetrics":{"sourceReads":{format!("{i}.rs"):1},"counters":{"x":1}}}),
            );
        }
        total.write(&mut report);
        assert_eq!(report["profileMetrics"]["uniqueSourcePathsRecorded"], 1024);
        assert_eq!(report["profileMetrics"]["sourcePathsOmittedAttempts"], 6);
    }
}
