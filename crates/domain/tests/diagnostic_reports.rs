use harness_domain::diagnostics::{append_check, summarize_checks};
use serde_json::json;

#[test]
fn report_health_preserves_error_priority_and_skipped_counts() {
    let (counts, health) = summarize_checks(&[
        json!({"status":"warning"}),
        json!({"status":"skipped"}),
        json!({"status":"error"}),
    ]);
    assert_eq!(health, "error");
    assert_eq!(counts, json!({"ok":0,"warning":1,"error":1,"skipped":1}));
}

#[test]
fn adding_protocol_checks_recomputes_the_shared_report_without_losing_checks() {
    let mut report = json!({"checks":[{"id":"runtime","status":"ok"}],"health":"ok"});
    append_check(&mut report, json!({"id":"registry","status":"warning"}));
    assert_eq!(report["checks"].as_array().unwrap().len(), 2);
    assert_eq!(report["health"], "warning");
    append_check(&mut report, json!({"id":"missing-status"}));
    assert_eq!(report["health"], "error");
    assert_eq!(report["counts"]["error"], 1);
}
