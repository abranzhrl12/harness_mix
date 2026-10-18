//! Background verification requests; process execution remains in platform adapters.
use crate::contracts::VerifyArgs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskArgs {
    /// start, status, list, events or cancel.
    pub action: String,
    /// Required by start. Uses the same bounded verification commands as harness_verify.
    pub verify: Option<VerifyArgs>,
    /// Required by status and cancel; optional filter for events.
    pub task_id: Option<String>,
    /// Return events after this sequence number.
    pub after: Option<u64>,
    /// Optional execution deadline in seconds. Omit for no execution deadline.
    pub timeout_seconds: Option<u64>,
    /// Total attempts, including the first: defaults to one, at most three.
    pub max_attempts: Option<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_unknown_fields_at_both_request_levels() {
        assert!(
            serde_json::from_value::<TaskArgs>(json!({
                "action":"start", "command":"arbitrary shell text"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<TaskArgs>(json!({
                "action":"start", "verify":{"language":"rust", "command":"test"}
            }))
            .is_err()
        );
    }

    #[test]
    fn accepts_camel_case_and_leaves_execution_deadline_optional() {
        let args: TaskArgs = serde_json::from_value(json!({
            "action":"start", "verify":{"language":"rust"}, "maxAttempts":2
        }))
        .unwrap();
        assert_eq!(args.max_attempts, Some(2));
        assert!(args.timeout_seconds.is_none());
        let args: TaskArgs = serde_json::from_value(json!({
            "action":"status", "taskId":"job-1", "after":7, "timeoutSeconds":60
        }))
        .unwrap();
        assert_eq!(args.task_id.as_deref(), Some("job-1"));
        assert_eq!(args.after, Some(7));
        assert_eq!(args.timeout_seconds, Some(60));
    }
}
