//! Derive response-only local coverage. Never certify unobserved inputs as current.
use serde_json::{Value, json};

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
            // Even identical content cannot prove the environment/toolchain unchanged.
            check["current"] = Value::Null;
            check["localFreshness"] = local;
        }
    }
    data["freshness"] = json!({
        "available":false,"reason":"full_input_coverage_not_tracked",
        "localObservation":observation,"derivedForThisResponse":true,
        "historicalChecksRevalidated":false
    });
}

#[cfg(test)]
mod tests {
    use super::*;
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
                assert!(value["observedByHarness"]["checks"][0]["current"].is_null());
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
