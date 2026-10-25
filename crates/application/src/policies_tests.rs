use crate::policies::{authorize, finish};
use harness_domain::{
    contracts::{ReadArgs, Request},
    execution::OperationContext,
    tool_contract::ToolProfile,
};
use serde_json::json;

fn read_request() -> Request {
    Request::Read(ReadArgs {
        external_scope: None,
        path: "README.md".into(),
        start: 1,
        end: 1,
        compact: false,
        line_numbers: false,
        session: None,
    })
}

#[test]
fn denied_profile_wins_over_cancellation_before_service_access() {
    let context = OperationContext::with_timeout(std::time::Duration::ZERO);
    let request = Request::Verify(serde_json::from_value(json!({"language": "rust"})).unwrap());
    let failure = authorize(ToolProfile::Investigate, &request, &context).unwrap_err();
    assert_eq!(failure.0, "TOOL_PROFILE");
}

#[test]
fn authorization_rejects_cancelled_requests_before_service_access() {
    let context = OperationContext::without_deadline();
    context.cancel();
    let failure = authorize(ToolProfile::Full, &read_request(), &context).unwrap_err();
    assert_eq!(failure.0, "CANCELLED");
}

#[test]
fn finish_preserves_partial_read_results_and_checks_complete_results() {
    let context = OperationContext::with_timeout(std::time::Duration::ZERO);
    let partial = json!({"cancelled": true, "matches": []});
    assert_eq!(finish(false, partial.clone(), &context).unwrap(), partial);
    let timed_out = json!({"timedOut": true, "matches": []});
    assert_eq!(
        finish(false, timed_out.clone(), &context).unwrap(),
        timed_out
    );
    assert_eq!(
        finish(false, json!({"ok": true}), &context).unwrap_err().0,
        "TIMEOUT"
    );
}

#[test]
fn finish_does_not_reclassify_a_completed_mutation_after_commit() {
    let context = OperationContext::with_timeout(std::time::Duration::ZERO);
    let committed = json!({"written": true});
    assert_eq!(
        finish(true, committed.clone(), &context).unwrap(),
        committed
    );
}

#[test]
fn policy_boundary_keeps_the_callers_progress_and_cancellation_context() {
    use std::sync::{Arc, Mutex};

    let updates = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&updates);
    let context = OperationContext::without_deadline().with_progress(move |update| {
        observed
            .lock()
            .unwrap()
            .push((update.phase, update.completed));
    });
    authorize(ToolProfile::Full, &read_request(), &context).unwrap();
    context.report_progress("reading", 1, Some(1), None);
    finish(false, json!({"ok": true}), &context).unwrap();
    assert_eq!(*updates.lock().unwrap(), vec![("reading", 1)]);
    context.cancel();
    assert_eq!(
        authorize(ToolProfile::Full, &read_request(), &context)
            .unwrap_err()
            .0,
        "CANCELLED"
    );
}
