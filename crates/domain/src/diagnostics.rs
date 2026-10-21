//! Pure severity aggregation shared by environment and protocol diagnostics.
use serde_json::{Value, json};

pub fn summarize_checks(checks: &[Value]) -> (Value, &'static str) {
    let mut counts = json!({"ok":0,"warning":0,"error":0,"skipped":0});
    for item in checks {
        let status = item["status"].as_str().unwrap_or("error");
        counts[status] = json!(counts[status].as_u64().unwrap_or(0) + 1);
    }
    let health = if counts["error"].as_u64().unwrap_or(0) > 0 {
        "error"
    } else if counts["warning"].as_u64().unwrap_or(0) > 0 {
        "warning"
    } else {
        "ok"
    };
    (counts, health)
}

pub fn append_check(report: &mut Value, item: Value) {
    let checks = report["checks"].as_array_mut().expect("doctor checks");
    checks.push(item);
    let (summary, health) = summarize_checks(checks);
    report["counts"] = summary;
    report["health"] = json!(health);
}
