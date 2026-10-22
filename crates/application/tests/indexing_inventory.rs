use harness_application::{
    indexing::{
        inventory::{self, Inventory, MAX_BYTES, MAX_VISITED},
        planning::{self, Observation, Plan},
    },
    ports::indexing::{DiscoveredEntry, InventoryPort, PlanningPort, SourceMetadata},
};
use harness_domain::{Failure, execution::OperationContext, hash};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap};

#[derive(Default)]
struct Fake {
    entries: Vec<(Option<String>, bool, bool)>,
    reads: RefCell<Vec<String>>,
    fail_read: bool,
    missing: bool,
    metadata_error: bool,
    size: u64,
    include: bool,
    stale: bool,
}
impl InventoryPort for Fake {
    fn scope(&self) -> Value {
        json!("scope")
    }
    fn has_includes(&self) -> bool {
        self.include
    }
    fn excluded(&self, path: &str) -> bool {
        path == "excluded.rs"
    }
    fn included(&self, path: &str) -> bool {
        path == "included.rs"
    }
    fn restricted_reason(&self, path: &str) -> Option<&'static str> {
        (path == "secret.rs").then_some("secret")
    }
    fn supported(&self, path: &str) -> bool {
        path.ends_with(".rs")
    }
    fn walk(&self, include: bool) -> Box<dyn Iterator<Item = DiscoveredEntry>> {
        let entries = if include {
            vec![(Some("included.rs".into()), true, false)]
        } else {
            self.entries.clone()
        };
        Box::new(
            entries
                .into_iter()
                .map(|(path, is_file, scan_error)| DiscoveredEntry {
                    path,
                    is_file,
                    scan_error,
                }),
        )
    }
    fn metadata(&self, _: &str) -> Result<SourceMetadata, (String, String)> {
        if self.metadata_error {
            Err(("IO_METADATA".into(), "denied".into()))
        } else if self.missing {
            Ok(SourceMetadata::Missing)
        } else {
            Ok(SourceMetadata::Regular(self.size))
        }
    }
    fn read(&self, path: &str, limit: u64, _: &OperationContext) -> Result<Vec<u8>, Failure> {
        assert_eq!(limit, self.size);
        self.reads.borrow_mut().push(path.into());
        if self.fail_read {
            Err(("IO", "denied"))
        } else {
            Ok(vec![b'x'; limit as usize])
        }
    }
    fn recheck(&self, _: &OperationContext) -> Result<(), Failure> {
        if self.stale {
            Err(("STALE", "scope changed"))
        } else {
            Ok(())
        }
    }
}
fn scan(f: &Fake, tracked: &[&str]) -> Inventory {
    inventory::scan(
        f,
        &tracked.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        &OperationContext::default(),
    )
    .unwrap()
}
#[test]
fn tracked_files_outside_discovery_are_observed_but_restricted_and_excluded_are_not_read() {
    let f = Fake::default();
    let r = scan(&f, &["tracked.rs", "secret.rs", "excluded.rs"]);
    assert_eq!(*f.reads.borrow(), ["tracked.rs"]);
    assert_eq!(r.restricted_indexed_files, 1);
    assert_eq!(r.scope_excluded, ["excluded.rs"]);
    assert!(!serde_json::to_string(&r).unwrap().contains("secret.rs"));
}
#[test]
fn explicit_missing_is_different_from_metadata_or_read_failure() {
    let f = Fake {
        missing: true,
        ..Fake::default()
    };
    assert!(matches!(
        scan(&f, &["a.rs"]).observations["a.rs"],
        Observation::Missing
    ));
    for f in [
        Fake {
            metadata_error: true,
            ..Fake::default()
        },
        Fake {
            fail_read: true,
            ..Fake::default()
        },
    ] {
        let r = scan(&f, &["a.rs"]);
        assert!(matches!(r.observations["a.rs"], Observation::Unchecked));
        assert_eq!(r.file_diagnostics.len(), 1);
    }
}
#[test]
fn discovery_errors_invalid_paths_and_include_pass_are_preserved() {
    let f = Fake {
        include: true,
        entries: vec![
            (None, false, true),
            (None, true, false),
            (Some("first.rs".into()), true, true),
        ],
        ..Fake::default()
    };
    let r = scan(&f, &[]);
    assert!(!r.discovery_complete);
    assert_eq!(r.skipped_entries, 3);
    assert!(r.observations.contains_key("first.rs"));
    assert!(r.observations.contains_key("included.rs"));
}
#[test]
fn lazy_discovery_stops_at_visit_budget() {
    let f = Fake {
        entries: vec![(None, false, false); MAX_VISITED + 10],
        ..Fake::default()
    };
    let r = scan(&f, &[]);
    assert_eq!(r.visited_entries, MAX_VISITED);
    assert!(r.budget_exhausted);
    assert!(!r.discovery_complete);
}
#[test]
fn aggregate_budget_reserves_reads_and_never_turns_overbudget_into_missing() {
    let f = Fake {
        size: 1024 * 1024,
        ..Fake::default()
    };
    let tracked = (0..65).map(|i| format!("{i}.rs")).collect::<Vec<_>>();
    let r = inventory::scan(&f, &tracked, &OperationContext::default()).unwrap();
    assert_eq!(r.reserved_source_bytes, MAX_BYTES);
    assert_eq!(f.reads.borrow().len(), 64);
    assert!(matches!(r.observations["64.rs"], Observation::Unchecked));
    assert!(r.budget_exhausted);
    assert!(!r.discovery_complete);
}
#[test]
fn final_scope_revalidation_failure_aborts_inventory() {
    let f = Fake {
        stale: true,
        ..Fake::default()
    };
    assert_eq!(
        inventory::scan(&f, &[], &OperationContext::default())
            .err()
            .unwrap()
            .0,
        "STALE"
    );
}

#[test]
fn cancelled_empty_inventory_cannot_succeed_or_read_sources() {
    let f = Fake::default();
    let context = OperationContext::default();
    context.cancel();
    assert_eq!(
        inventory::scan(&f, &[], &context).err().unwrap().0,
        "CANCELLED"
    );
    assert!(f.reads.borrow().is_empty());
}
#[test]
fn pure_classification_handles_all_categories_without_sql() {
    let indexed = ["modified", "retry", "unchanged", "missing", "unchecked"]
        .into_iter()
        .map(|p| (format!("{p}.rs"), (hash(b"old"), p != "retry")))
        .collect();
    let observed = BTreeMap::from([
        ("modified.rs".into(), Observation::Content(hash(b"new"))),
        ("retry.rs".into(), Observation::Content(hash(b"old"))),
        ("unchanged.rs".into(), Observation::Content(hash(b"old"))),
        ("missing.rs".into(), Observation::Missing),
        ("unchecked.rs".into(), Observation::Unchecked),
        ("added.rs".into(), Observation::Content(hash(b"new"))),
    ]);
    let r = planning::classify(&indexed, &observed, false, &OperationContext::default()).unwrap();
    assert_eq!(r.added, ["added.rs"]);
    assert_eq!(r.modified, ["modified.rs"]);
    assert_eq!(r.retry_extraction, ["retry.rs"]);
    assert_eq!(r.unchanged, ["unchanged.rs"]);
    assert_eq!(r.removed, ["missing.rs"]);
    assert_eq!(r.unchecked, ["unchecked.rs"]);
}
#[test]
fn incomplete_or_unreadable_observations_never_infer_deletion() {
    let indexed = BTreeMap::from([("a.rs".into(), (hash(b""), true))]);
    let r = planning::classify(
        &indexed,
        &BTreeMap::new(),
        false,
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(r.unchecked, ["a.rs"]);
    assert!(r.removed.is_empty());
    let observed = BTreeMap::from([("a.rs".into(), Observation::Unchecked)]);
    let r = planning::classify(&indexed, &observed, true, &OperationContext::default()).unwrap();
    assert_eq!(r.unchecked, ["a.rs"]);
    assert!(r.removed.is_empty());
}

struct Planner {
    calls: Vec<&'static str>,
    fail: Option<&'static str>,
}
impl PlanningPort for Planner {
    fn indexed(
        &mut self,
        _: &OperationContext,
    ) -> Result<BTreeMap<String, (String, bool)>, Failure> {
        self.calls.push("indexed");
        if self.fail == Some("indexed") {
            return Err(("TEST", "indexed"));
        }
        Ok(BTreeMap::from([
            ("excluded.rs".into(), (hash(b""), true)),
            ("secret.rs".into(), (hash(b""), true)),
        ]))
    }
    fn inventory(
        &mut self,
        tracked: &[String],
        _: &OperationContext,
    ) -> Result<Inventory, Failure> {
        self.calls.push("inventory");
        assert_eq!(tracked, ["excluded.rs", "secret.rs"]);
        if self.fail == Some("inventory") {
            return Err(("TEST", "inventory"));
        }
        Ok(scan(&Fake::default(), &["excluded.rs", "secret.rs"]))
    }
    fn restricted(&self, p: &str) -> bool {
        p == "secret.rs"
    }
    fn preview(
        &mut self,
        plan: &Plan,
        _: &Inventory,
        _: &OperationContext,
    ) -> Result<Value, Failure> {
        self.calls.push("preview");
        assert_eq!(plan.removed, ["excluded.rs"]);
        assert!(plan.unchecked.is_empty());
        if self.fail == Some("preview") {
            Err(("TEST", "preview"))
        } else {
            Ok(json!("preview"))
        }
    }
}
#[test]
fn planning_coordinates_snapshot_inventory_exclusions_and_optional_preview() {
    for preview in [false, true] {
        let mut p = Planner {
            calls: vec![],
            fail: None,
        };
        let (plan, _, extraction) =
            planning::plan_workspace(&mut p, preview, &OperationContext::default()).unwrap();
        assert_eq!(plan.removed, ["excluded.rs"]);
        assert!(plan.unchecked.is_empty());
        assert_eq!(
            p.calls,
            if preview {
                vec!["indexed", "inventory", "preview"]
            } else {
                vec!["indexed", "inventory"]
            }
        );
        assert_eq!(extraction.is_null(), !preview);
    }
}
#[test]
fn planning_failures_stop_before_next_port() {
    for stage in ["indexed", "inventory", "preview"] {
        let mut p = Planner {
            calls: vec![],
            fail: Some(stage),
        };
        assert_eq!(
            planning::plan_workspace(&mut p, true, &OperationContext::default())
                .err()
                .unwrap(),
            ("TEST", stage)
        );
        assert_eq!(p.calls.last(), Some(&stage));
    }
}
