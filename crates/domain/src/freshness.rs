//! Local coverage and sticky invalidation. Never certify unobserved inputs as current.
use serde_json::{Value, json};

/// Invalidation is monotonic for an existing check; only a new run creates a new fact.
pub fn invalidate(check: &mut Value, reason: &str, time: &str) {
    if check["current"] != false {
        check["current"] = json!(false);
        check["invalidation"] = json!({"reason":reason,"time":time,"requiresRerun":true});
    }
}

pub fn invalidate_observed_checks(checks: &mut [Value], observation: &Value) {
    for check in checks {
        if assessment(check, observation)["status"] == "local_workspace_changed" {
            invalidate(
                check,
                "local_workspace_changed",
                observation["capturedAt"].as_str().unwrap_or(""),
            );
        }
    }
}

fn assessment(check: &Value, observation: &Value) -> Value {
    let baseline = &check["workspaceObservation"];
    let captured = observation["status"] == "captured" && observation["hash"].is_string();
    let stable = baseline["status"] == "unchanged"
        && baseline["coverage"].is_object()
        && baseline["coverage"] == observation["coverage"]
        && baseline["before"].is_string()
        && baseline["before"] == baseline["after"];
    let status = if !captured {
        "unknown"
    } else if !stable {
        "no_stable_baseline"
    } else if baseline["after"] == observation["hash"] {
        "matches_local_workspace"
    } else {
        "local_workspace_changed"
    };
    json!({"status":status,"checkedAt":observation["capturedAt"],"certifiesCurrent":false})
}

pub fn refresh_checkpoint(data: &mut Value, observation: &Value) {
    // Apply equally to compact and full detail. Original stored facts remain immutable.
    if let Some(full) = data.get_mut("_detail") {
        refresh_checkpoint(full, observation);
    }
    if let Some(checks) = data["observedByHarness"]["checks"].as_array_mut() {
        for check in checks {
            let local = assessment(check, observation);
            if local["status"] == "local_workspace_changed" {
                invalidate(
                    check,
                    "local_workspace_changed",
                    observation["capturedAt"].as_str().unwrap_or(""),
                );
            }
            // Even identical content cannot prove the environment/toolchain unchanged.
            if check["current"] != false {
                check["current"] = Value::Null;
            }
            check["localFreshness"] = local;
        }
    }
    data["freshness"] = json!({
        "available":false,"reason":"full_input_coverage_not_tracked",
        "localObservation":observation,"derivedForThisResponse":true,
        "historicalChecksRevalidated":false,"invalidationTracked":true
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalidation_survives_reversion_unknown_scans_and_detail_projection() {
        let baseline = json!({"status":"unchanged","before":"a","after":"a","coverage":{}});
        let mut checks = vec![json!({"current":null,"workspaceObservation":baseline})];
        let observed = |status: &str, hash: &str| json!({"status":status,"hash":hash,"capturedAt":"now","coverage":{}});
        // Incompatible coverage and unknown observations cannot prove a change.
        let mut incompatible = observed("captured", "b");
        incompatible["coverage"] = json!({"kind":"other"});
        invalidate_observed_checks(&mut checks, &incompatible);
        invalidate_observed_checks(&mut checks, &observed("unknown", "b"));
        assert!(checks[0]["current"].is_null());
        invalidate_observed_checks(&mut checks, &observed("captured", "b"));
        assert_eq!(checks[0]["current"], false);
        let first_reason = checks[0]["invalidation"].clone();
        invalidate(&mut checks[0], "another_change", "later");
        assert_eq!(checks[0]["invalidation"], first_reason);
        for observation in [observed("captured", "a"), observed("unknown", "a")] {
            let mut data = json!({"observedByHarness":{"checks":checks}});
            data["_detail"] = data.clone();
            refresh_checkpoint(&mut data, &observation);
            for projected in [&data, &data["_detail"]] {
                assert_eq!(
                    projected["observedByHarness"]["checks"][0]["current"],
                    false
                );
                assert_eq!(
                    projected["observedByHarness"]["checks"][0]["invalidation"],
                    first_reason
                );
            }
        }
    }
    #[test]
    fn matching_mismatched_missing_and_unstable_baselines_never_certify_current() {
        let current = json!({"status":"captured","hash":"a","capturedAt":"now","coverage":{}});
        for (mut baseline, status) in [
            (
                json!({"status":"unchanged","before":"a","after":"a"}),
                "matches_local_workspace",
            ),
            (
                json!({"status":"unchanged","before":"b","after":"b"}),
                "local_workspace_changed",
            ),
            (
                json!({"status":"changed","before":"b","after":"a"}),
                "no_stable_baseline",
            ),
            (Value::Null, "no_stable_baseline"),
        ] {
            if baseline.is_object() {
                baseline["coverage"] = json!({});
            }
            let mut data = json!({"observedByHarness":{"checks":[{"current":true,"workspaceObservation":baseline}]}});
            data["_detail"] = data.clone();
            refresh_checkpoint(&mut data, &current);
            for value in [&data, &data["_detail"]] {
                assert_eq!(
                    value["observedByHarness"]["checks"][0]["current"],
                    if status == "local_workspace_changed" {
                        json!(false)
                    } else {
                        Value::Null
                    }
                );
                assert_eq!(
                    value["observedByHarness"]["checks"][0]["localFreshness"]["status"],
                    status
                );
            }
        }
        assert_eq!(
            assessment(&json!({}), &json!({"status":"unknown"}))["status"],
            "unknown"
        );
    }
}
