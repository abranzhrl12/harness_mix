use harness_domain::Failure;
use serde_json::{Value, json};
use std::time::Duration;

// Keep a bounded diagnostic report instead of discarding the reason a full
// reconciliation was incomplete. File bodies and per-file reports stay out.
pub(super) fn sync_report(result: &Result<Value, Failure>, full: bool) -> Value {
    let mut report = json!({"complete":false,"retryable":false,"failure":null});
    match result {
        Ok(value) => {
            for key in [
                "selectedScopeApplied",
                "sourceScopeApplied",
                "warningFiles",
                "indexArchive",
                "discoveryComplete",
                "uncheckedFiles",
                "unsupportedChangedFiles",
                "restrictedIndexedFiles",
                "restrictedIndexedReasons",
                "nativeFilesRemaining",
                "filesProcessed",
                "changedFilesCommitted",
            ] {
                if let Some(value) = value.get(key) {
                    report[key] = value.clone();
                }
            }
            if let Some(failure) = value.get("failure").filter(|v| !v.is_null()) {
                report["failure"] = failure.clone();
            }
            if let Some(warnings) = value["warnings"].as_array() {
                report["warnings"] = json!(warnings.iter().take(32).collect::<Vec<_>>());
            }
            report["complete"] = json!(
                value["selectedScopeApplied"] == true
                    && report["failure"].is_null()
                    && (!full
                        || (value["discoveryComplete"] == true
                            && value["uncheckedFiles"] == 0
                            && value["unsupportedChangedFiles"] == 0))
            );
        }
        Err((code, message)) => {
            report["failure"] = json!({"code":code,"message":message});
        }
    }
    let sources_applied = report["sourceScopeApplied"] == true
        && report["failure"].is_null()
        && (!full
            || (report["discoveryComplete"] == true
                && report["uncheckedFiles"] == 0
                && report["unsupportedChangedFiles"] == 0));
    if report["complete"] != true && !sources_applied {
        let code = report["failure"]["code"]
            .as_str()
            .unwrap_or("SYNC_INCOMPLETE");
        // Coverage gaps and deterministic incompatibilities require a change,
        // not another identical full inventory after a short backoff.
        let retryable = !matches!(
            code,
            "SYNC_INCOMPLETE"
                | "SYNC_LANGUAGE"
                | "SYNC_UNSUPPORTED_RELATION"
                | "SYNC_LEGACY_REFERENCE"
                | "SYNC_SCHEMA"
                | "SYNC_CONFIG"
                | "SYNC_LIMIT"
                | "SYNC_INTEGRITY"
                | "SYNC_COMMIT_UNCERTAIN"
                | "ENCODING"
                | "PATH"
                | "LIMIT"
                | "CANCELLED"
        );
        report["lastError"] = json!(code);
        report["retryable"] = json!(retryable);
    } else {
        report["lastError"] = Value::Null;
        if report["complete"] != true {
            report["graphStatus"] = json!("partial");
        }
    }
    report
}

// At most five attempts between source events/periodic reconciliations.
pub(super) fn retry_delay(report: &Value, failures: u32) -> Option<Duration> {
    (report["retryable"] == true && (1..5).contains(&failures))
        .then(|| Duration::from_secs(5_u64 << (failures - 1)))
}
