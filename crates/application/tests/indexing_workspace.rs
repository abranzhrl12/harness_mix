use harness_application::{
    indexing::{changed_paths, workspace},
    ports::indexing::{ChangedPathsPort, WorkspacePort},
};
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

struct Fake {
    plan: Value,
    applied: Vec<String>,
    waves: Vec<Vec<String>>,
    failures: BTreeMap<String, Failure>,
    start_failure: Option<&'static str>,
    final_failure: bool,
    workers: usize,
    fallback: bool,
    partial_preparation: bool,
    isolated_failure: bool,
}
impl Fake {
    fn new(paths: &[&str]) -> Self {
        Self {
            plan: json!({"scope":"v1","plan":{"added":paths,"modified":[],"removed":[],"retryExtraction":[],"unchanged":[],"unchecked":[]},"inventory":{}}),
            applied: vec![],
            waves: vec![],
            failures: BTreeMap::new(),
            start_failure: None,
            final_failure: false,
            workers: 4,
            fallback: false,
            partial_preparation: false,
            isolated_failure: true,
        }
    }
}
impl WorkspacePort for Fake {
    type Prepared = String;
    fn load_scope(&mut self, _: &OperationContext) -> Result<Value, Failure> {
        if self.start_failure == Some("scope") {
            Err(("TEST", "scope"))
        } else {
            Ok(json!("v1"))
        }
    }
    fn plan(&mut self, _: &OperationContext) -> Result<Value, Failure> {
        if self.start_failure == Some("plan") {
            Err(("TEST", "plan"))
        } else {
            Ok(self.plan.clone())
        }
    }
    fn language(&self, path: &str) -> Option<&'static str> {
        if path.ends_with(".rs") {
            Some("rust")
        } else if path.ends_with(".ts") {
            Some("typescript")
        } else {
            None
        }
    }
    fn workers(&self) -> usize {
        self.workers
    }
    fn prepare(&mut self, paths: &[&String], _: &OperationContext) -> Vec<Option<String>> {
        self.waves
            .push(paths.iter().map(|p| (*p).clone()).collect());
        if self.partial_preparation {
            return vec![];
        }
        paths.iter().map(|p| Some((*p).clone())).collect()
    }
    fn apply(
        &mut self,
        path: &str,
        prepared: Option<String>,
        force: bool,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        if self.workers > 1 && !self.partial_preparation {
            assert_eq!(prepared.as_deref(), Some(path));
        }
        self.applied.push(path.into());
        if let Some(f) = self.failures.get(path) {
            return Err(*f);
        }
        if self.start_failure == Some("cancel_file") {
            c.cancel();
            return c.check().map(|_| Value::Null);
        }
        Ok(json!({"path":path,"changed":true,"committed":true,"forced":force}))
    }
    fn diagnostic(&mut self, _: &str, _: &str) -> Option<Value> {
        Some(json!({"stage":"fake"}))
    }
    fn file_failure_is_isolated(&self) -> bool {
        self.isolated_failure
    }
    fn recheck(&mut self, _: &OperationContext) -> Result<(), Failure> {
        if self.final_failure {
            Err(("STALE", "scope changed"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn single_worker_bypasses_optional_preparation() {
    let mut f = Fake::new(&["a.rs", "b.rs"]);
    f.workers = 1;
    let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 20, false).unwrap();
    assert!(f.waves.is_empty());
    assert_eq!(r["changedFilesCommitted"], 2);
}

#[test]
fn partial_preparation_falls_back_without_shifting_file_slots() {
    let mut f = Fake::new(&["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"]);
    f.partial_preparation = true;
    let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 20, false).unwrap();
    assert_eq!(f.applied, ["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"]);
    assert_eq!(r["changedFilesCommitted"], 5);
}

#[test]
fn workspace_cancellation_stops_after_current_file_without_later_publications() {
    let mut f = Fake::new(&["a.rs", "b.rs"]);
    f.start_failure = Some("cancel_file");
    let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 20, false).unwrap();
    assert_eq!(f.applied, ["a.rs"]);
    assert_eq!(r["failure"]["code"], "CANCELLED");
    assert_eq!(r["changedFilesCommitted"], 0);
}
impl ChangedPathsPort for Fake {
    fn file_failure_is_isolated(&self) -> bool {
        self.isolated_failure
    }
    fn initialize(&mut self, _: &OperationContext) -> Result<(), Failure> {
        if self.start_failure.is_some() {
            Err(("TEST", "init"))
        } else {
            Ok(())
        }
    }
    fn needs_inventory(&self, _: &BTreeSet<String>) -> Result<bool, Failure> {
        Ok(self.fallback)
    }
    fn sync_workspace(&mut self, _: &OperationContext) -> Result<Value, Failure> {
        Ok(json!({"fallback":true}))
    }
    fn apply_changed(
        &mut self,
        path: &str,
        _: &OperationContext,
    ) -> Result<Option<Value>, Failure> {
        self.applied.push(path.into());
        if let Some(f) = self.failures.get(path) {
            return Err(*f);
        }
        if path.ends_with(".skip") {
            Ok(None)
        } else {
            Ok(Some(json!({"changed":true})))
        }
    }
}
#[test]
fn scheduling_is_sorted_deduplicated_bounded_and_preparation_stays_positional() {
    let mut f = Fake::new(&["e.rs", "b.rs", "a.rs", "c.rs", "d.rs", "a.rs"]);
    f.workers = 99;
    let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 5, false).unwrap();
    assert_eq!(f.applied, ["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"]);
    assert_eq!(f.waves.iter().map(Vec::len).collect::<Vec<_>>(), [4, 1]);
    assert_eq!(r["changedFilesCommitted"], 5);
    assert_eq!(r["extractionWorkers"], 4);
}
#[test]
fn local_failure_keeps_confirmed_commits_and_continues() {
    let mut f = Fake::new(&["a.rs", "b.rs", "c.rs"]);
    f.failures.insert("b.rs".into(), ("SQL", "injected"));
    let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 20, false).unwrap();
    assert_eq!(f.applied.len(), 3);
    assert_eq!(r["changedFilesCommitted"], 2);
    assert_eq!(r["failedFiles"], 1);
    assert_eq!(r["failure"]["diagnostic"]["stage"], "fake");
    assert_eq!(r["selectedScopeApplied"], false);
}

#[test]
fn legacy_script_semantic_failure_is_reported_as_warning_and_does_not_stop_workspace() {
    let mut f = Fake::new(&["a.ts", "b.ts", "c.ts"]);
    f.failures
        .insert("b.ts".into(), ("SYNC_SEMANTIC", "outside native semantic subset"));
    let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 20, false).unwrap();
    assert_eq!(f.applied, ["a.ts", "b.ts", "c.ts"]);
    assert_eq!(r["changedFilesCommitted"], 2);
    assert_eq!(r["failedFiles"], 1);
    assert_eq!(r["warningFiles"], 1);
    assert_eq!(r["warnings"][0]["path"], "b.ts");
    assert_eq!(r["warnings"][0]["severity"], "warning");
    assert_eq!(r["warnings"][0]["indexStatus"], "not_updated");
    assert_eq!(r["stoppedEarly"], false);
    assert_eq!(r["selectedScopeApplied"], false);
}
#[test]
fn all_systemic_failure_codes_stop_later_publications() {
    for code in [
        "SYNC_SCHEMA",
        "SYNC_INDEX",
        "SYNC_INDEX_BUSY",
        "SYNC_INTEGRITY",
        "SYNC_COMMIT_UNCERTAIN",
        "PATH",
    ] {
        let mut f = Fake::new(&["a.rs", "b.rs"]);
        f.failures.insert("a.rs".into(), (code, "injected"));
        let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 20, false).unwrap();
        assert_eq!(f.applied, ["a.rs"]);
        assert_eq!(r["failedFiles"], 1);
        assert_eq!(
            r["failure"]["commitState"],
            if code == "SYNC_COMMIT_UNCERTAIN" {
                "unknown"
            } else {
                "not_committed"
            }
        );
    }
}
#[test]
fn isolated_budget_and_source_failures_do_not_starve_other_files() {
    for code in [
        "SYNC_LIMIT",
        "LIMIT",
        "PARSER_LIMIT",
        "SYNTAX_ERROR",
        "ENCODING",
        "STALE",
        "TIMEOUT",
    ] {
        let mut f = Fake::new(&["a.rs", "b.rs", "c.rs"]);
        f.failures.insert("b.rs".into(), (code, "local failure"));
        let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 20, false).unwrap();
        assert_eq!(f.applied, ["a.rs", "b.rs", "c.rs"]);
        assert_eq!(r["changedFilesCommitted"], 2);
        assert_eq!(r["failedFiles"], 1);
        assert_eq!(r["nativeFilesRemaining"], 1);
        assert_eq!(r["unattemptedFiles"], 0);
        assert_eq!(r["stoppedEarly"], false);
        assert_eq!(r["selectedScopeApplied"], false);
    }
}

#[test]
fn workspace_precondition_failure_stops_even_with_a_locally_recoverable_code() {
    for code in ["STALE", "SYNC_LIMIT"] {
        let mut f = Fake::new(&["a.rs", "b.rs"]);
        f.isolated_failure = false;
        f.failures
            .insert("a.rs".into(), (code, "workspace precondition"));
        let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 20, false).unwrap();
        assert_eq!(f.applied, ["a.rs"]);
        assert_eq!(r["stoppedEarly"], true);
        assert_eq!(r["unattemptedFiles"], 1);
    }
}

#[test]
fn many_local_failures_are_bounded_without_losing_successes_or_error_counts() {
    let paths: Vec<String> = (0..80)
        .map(|n| format!("bad-{n:03}.rs"))
        .chain(std::iter::once("z-good.rs".into()))
        .collect();
    let names: Vec<&str> = paths.iter().map(String::as_str).collect();
    let mut f = Fake::new(&names);
    for path in &paths[..80] {
        f.failures
            .insert(path.clone(), ("SYNC_LIMIT", "local budget"));
    }
    let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 100, false).unwrap();
    assert_eq!(f.applied.len(), 81);
    assert_eq!(r["changedFilesCommitted"], 1);
    assert_eq!(r["failedFiles"], 80);
    assert_eq!(r["failures"].as_array().unwrap().len(), 32);
    assert_eq!(r["failureReportsOmitted"], 48);
    assert_eq!(r["filesAttempted"], 81);
    assert_eq!(r["unattemptedFiles"], 0);
    assert_eq!(r["nativeFilesRemaining"], 80);
    assert_eq!(r["selectedScopeApplied"], false);
}

#[test]
fn final_scope_failure_is_not_hidden_by_an_earlier_local_error() {
    let mut f = Fake::new(&["a.rs", "b.rs"]);
    f.failures
        .insert("a.rs".into(), ("SYNC_LIMIT", "local budget"));
    f.final_failure = true;
    let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 20, false).unwrap();
    assert_eq!(r["changedFilesCommitted"], 1);
    assert_eq!(r["failures"].as_array().unwrap().len(), 2);
    assert_eq!(r["failures"][1]["stage"], "final_scope_recheck");
    assert_eq!(r["stoppedEarly"], true);
    assert_eq!(r["selectedScopeApplied"], false);
}

#[test]
fn provider_skip_is_reported_without_aborting_next_file() {
    let mut f = Fake::new(&["a.rs", "b.rs"]);
    f.failures
        .insert("a.rs".into(), ("SYNC_UNSUPPORTED_RELATION", "provider"));
    let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 20, false).unwrap();
    assert_eq!(r["skippedFiles"], 1);
    assert_eq!(r["changedFilesCommitted"], 1);
    assert_eq!(r["selectedScopeApplied"], false);
    assert!(r["failure"].is_null());
}
#[test]
fn scope_or_plan_failure_and_stale_plan_never_publish() {
    for stage in ["scope", "plan", "stale"] {
        let mut f = Fake::new(&["a.rs"]);
        if stage == "stale" {
            f.plan["scope"] = json!("v2");
        } else {
            f.start_failure = Some(stage);
        }
        assert!(
            workspace::sync_workspace(&mut f, &OperationContext::default(), 20, false).is_err()
        );
        assert!(f.applied.is_empty());
    }
}
#[test]
fn final_scope_failure_does_not_hide_earlier_commits() {
    let mut f = Fake::new(&["a.rs"]);
    f.final_failure = true;
    let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 20, false).unwrap();
    assert_eq!(r["changedFilesCommitted"], 1);
    assert_eq!(r["failure"]["stage"], "final_scope_recheck");
}
#[test]
fn report_cap_does_not_cap_commit_accounting() {
    let names = (0..40).map(|i| format!("{i:02}.rs")).collect::<Vec<_>>();
    let mut f = Fake::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
    let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 40, false).unwrap();
    assert_eq!(r["files"].as_array().unwrap().len(), 32);
    assert_eq!(r["changedFilesCommitted"], 40);
    assert_eq!(r["fileReportsOmitted"], 8);
}
#[test]
fn bounded_run_leaves_remaining_work_and_rebuild_selects_unchanged() {
    let mut f = Fake::new(&["a.rs", "b.rs"]);
    f.plan["plan"]["unchanged"] = json!(["c.rs"]);
    let r = workspace::sync_workspace(&mut f, &OperationContext::default(), 1, true).unwrap();
    assert_eq!(r["nativeCandidates"], 3);
    assert_eq!(r["nativeFilesRemaining"], 2);
    assert_eq!(r["files"][0]["forced"], true);
}
#[test]
fn no_changes_revisits_one_module_to_detect_configuration_changes() {
    let mut f = Fake::new(&[]);
    f.plan["plan"]["unchanged"] = json!(["a.rs", "b.ts", "c.ts"]);
    workspace::sync_workspace(&mut f, &OperationContext::default(), 20, false).unwrap();
    assert_eq!(f.applied, ["b.ts"]);
}
#[test]
fn event_local_failure_preserves_prior_commits_and_continues() {
    let mut f = Fake::new(&[]);
    f.failures.insert("c.rs".into(), ("TEST", "error"));
    let paths = ["a.rs", "b.skip", "c.rs", "d.rs"]
        .map(String::from)
        .into_iter()
        .collect();
    let r =
        changed_paths::sync_changed_paths(&mut f, &paths, &OperationContext::default()).unwrap();
    assert_eq!(r["changedFilesCommitted"], 2);
    assert_eq!(r["skippedFiles"], 1);
    assert_eq!(r["failedFiles"], 1);
    assert_eq!(r["unattemptedFiles"], 0);
    assert_eq!(r["selectedScopeApplied"], false);
    assert_eq!(f.applied, ["a.rs", "b.skip", "c.rs", "d.rs"]);
}

#[test]
fn event_systemic_or_workspace_scope_failure_stops_later_files() {
    for code in [
        "SYNC_SCHEMA",
        "SYNC_INDEX",
        "SYNC_INDEX_BUSY",
        "SYNC_INTEGRITY",
        "SYNC_COMMIT_UNCERTAIN",
        "PATH",
        "STALE",
    ] {
        let mut f = Fake::new(&[]);
        f.isolated_failure = code != "STALE";
        f.failures.insert("a.rs".into(), (code, "systemic"));
        let paths = ["a.rs", "b.rs"].map(String::from).into_iter().collect();
        let r = changed_paths::sync_changed_paths(&mut f, &paths, &OperationContext::default())
            .unwrap();
        assert_eq!(f.applied, ["a.rs"]);
        assert_eq!(r["stoppedEarly"], true);
        assert_eq!(r["unattemptedFiles"], 1);
        assert_eq!(
            r["failure"]["commitState"],
            if code == "SYNC_COMMIT_UNCERTAIN" {
                "unknown"
            } else {
                "not_committed"
            }
        );
    }
}

#[test]
fn event_failure_reports_are_bounded_and_do_not_starve_good_files() {
    let paths: BTreeSet<String> = (0..70)
        .map(|n| format!("bad-{n:03}.rs"))
        .chain(std::iter::once("z-good.rs".into()))
        .collect();
    let mut f = Fake::new(&[]);
    for path in paths.iter().filter(|p| p.starts_with("bad-")) {
        f.failures
            .insert(path.clone(), ("SYNC_LIMIT", "local budget"));
    }
    let r =
        changed_paths::sync_changed_paths(&mut f, &paths, &OperationContext::default()).unwrap();
    assert_eq!(r["changedFilesCommitted"], 1);
    assert_eq!(r["failedFiles"], 70);
    assert_eq!(r["failures"].as_array().unwrap().len(), 32);
    assert_eq!(r["failureReportsOmitted"], 38);
    assert_eq!(r["unattemptedFiles"], 0);
    assert_eq!(r["selectedScopeApplied"], false);
}

#[test]
fn explicit_cancellation_code_stops_even_before_the_owner_flag_propagates() {
    let mut full = Fake::new(&["a.rs", "b.rs"]);
    full.failures
        .insert("a.rs".into(), ("CANCELLED", "cancelled"));
    let r = workspace::sync_workspace(&mut full, &OperationContext::default(), 20, false).unwrap();
    assert_eq!(full.applied, ["a.rs"]);
    assert_eq!(r["stoppedEarly"], true);
    let mut events = Fake::new(&[]);
    events
        .failures
        .insert("a.rs".into(), ("CANCELLED", "cancelled"));
    let paths = ["a.rs", "b.rs"].map(String::from).into_iter().collect();
    let r = changed_paths::sync_changed_paths(&mut events, &paths, &OperationContext::default())
        .unwrap();
    assert_eq!(events.applied, ["a.rs"]);
    assert_eq!(r["stoppedEarly"], true);
}
#[test]
fn event_worktree_fallback_uses_inventory_instead_of_publishing_blindly() {
    let mut f = Fake::new(&[]);
    f.fallback = true;
    let r =
        changed_paths::sync_changed_paths(&mut f, &BTreeSet::new(), &OperationContext::default())
            .unwrap();
    assert_eq!(r["fallback"], true);
    assert!(f.applied.is_empty());
}
#[test]
fn event_budget_is_checked_before_infrastructure() {
    let mut f = Fake::new(&[]);
    f.start_failure = Some("init");
    let paths = (0..1025).map(|i| format!("{i}.rs")).collect();
    assert_eq!(
        changed_paths::sync_changed_paths(&mut f, &paths, &OperationContext::default())
            .unwrap_err()
            .0,
        "SYNC_LIMIT"
    );
}
