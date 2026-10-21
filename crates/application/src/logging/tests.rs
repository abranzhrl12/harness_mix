use super::state::{Log, SLOW_MICROS};
use harness_domain::contracts::LogsArgs;
use serde_json::json;

#[test]
fn timeout_context_is_bounded_redacted_and_excluded_from_disk_archive() {
    let log = Log::configured(true, Ok(None));
    let id = harness_domain::execution::new_request_id();
    log.record_correlated(
        "harness_verify",
        "completed",
        Some("timeout"),
        Some("TIMEOUT"),
        Some(60_000_000),
        &id,
    );
    log.annotate(
        &id,
        &json!({"state":"timeout","scope":{"tool":"harness_verify","path":"crates/domain"},
            "data":{"command":{"program":"cargo","arguments":["check","-p","domain"]},
                "stdoutTail":"progress","stderrTail":"compiling domain\nDATABASE_URL=private-value\nlast cargo error"}}),
    );
    let value = log.read(&args()).unwrap();
    let event = &value["events"][0];
    assert_eq!(event["context"]["path"], "crates/domain");
    let tail = event["context"]["stderrTail"].as_str().unwrap();
    assert!(tail.contains("last cargo error"));
    assert!(!tail.contains("private-value"));
    let archive = log.archive().unwrap();
    harness_domain::log_archive::validate(&archive).unwrap();
    assert!(archive["events"][0].get("context").is_none());
}

fn args() -> LogsArgs {
    LogsArgs {
        limit: Some(2),
        level: None,
        tool: None,
        slow_only: None,
        request_id: None,
        after: Some(0),
        session: None,
    }
}

#[test]
fn delayed_flush_reports_cannot_hide_a_newer_storage_failure() {
    let log = Log::configured(true, Ok(None));
    log.saved(2, None);
    log.saved(3, Some("LOG_IO"));
    log.saved(2, None);
    assert_eq!(log.status()["persistenceError"], "LOG_IO");
    log.saved(4, None);
    log.saved(3, Some("LOG_IO"));
    assert!(log.status()["persistenceError"].is_null());
}

#[test]
fn bounded_history_cursor_and_sanitized_codes() {
    let log = Log::default();
    for _ in 0..514 {
        log.record(
            "harness_read",
            "completed",
            Some("error"),
            Some("credential=must-not-survive"),
            Some(12),
        );
    }
    let page = log.read(&args()).unwrap();
    assert_eq!(page["retained"], 512);
    assert_eq!(page["discarded"], 2);
    assert_eq!(page["historyPruned"], true);
    assert_eq!(page["nextAfter"], 4);
    assert_eq!(page["events"][0]["code"], "TOOL_ERROR");
    assert!(!page.to_string().contains("credential"));
    let mut filter = args();
    filter.tool = Some("harness_other".into());
    assert_eq!(log.read(&filter).unwrap()["nextAfter"], 514);
}

#[test]
fn level_tool_and_slow_filters_do_not_retain_arbitrary_values() {
    let log = Log::default();
    log.record(
        "harness_read",
        "completed",
        Some("ok"),
        None,
        Some(SLOW_MICROS),
    );
    log.record(
        "harness_search",
        "completed",
        Some("error"),
        Some("ARGUMENT"),
        Some(5),
    );
    let mut query = args();
    query.after = None;
    query.slow_only = Some(true);
    let page = log.read(&query).unwrap();
    assert_eq!(page["events"].as_array().unwrap().len(), 1);
    assert_eq!(page["events"][0]["tool"], "harness_read");
    query.level = Some("error".into());
    assert!(
        log.read(&query).unwrap()["events"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    query.after = Some(3);
    assert!(log.read(&query).is_err());
    query.after = None;
    query.limit = Some(0);
    assert!(log.read(&query).is_err());
}
