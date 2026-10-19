//! Background verification requests; process execution remains in platform adapters.
use crate::contracts::VerifyArgs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskArgs {
    /// start, status, list, events, cancel or observe.
    pub action: String,
    /// Required by start. Uses the same bounded verification commands as harness_verify.
    pub verify: Option<VerifyArgs>,
    /// Required by status, cancel and observe; optional filter for events.
    #[serde(alias = "job_id", alias = "jobId")]
    pub task_id: Option<String>,
    /// Return events after this sequence number (or context lines after match for observe).
    pub after: Option<u64>,
    /// Optional execution deadline in seconds. Omit for no execution deadline.
    pub timeout_seconds: Option<u64>,
    /// Total attempts, including the first: defaults to one, at most three.
    pub max_attempts: Option<u8>,
    /// Rust regex; omitted matches every retained line. Maximum 1024 bytes (for observe).
    pub filter: Option<String>,
    /// Maximum matching lines, 1..200; defaults to 20 (for observe).
    pub lines: Option<usize>,
    /// Context lines on each side, 0..20; defaults to 2 (for observe).
    pub before: Option<usize>,
    /// stdout, stderr or both (default) (for observe).
    pub stream: Option<String>,
    /// Only output events after this cursor (for observe).
    pub cursor: Option<u64>,
    /// UTF-8 byte ceiling, 256..16000; defaults to 6000 (for observe).
    #[serde(rename = "maxChars")]
    pub max_chars: Option<usize>,
}

impl TaskArgs {
    pub fn into_observe(&self) -> Result<crate::assistance::ObserveArgs, crate::Failure> {
        let task_id = self
            .task_id
            .as_deref()
            .ok_or(("ARGUMENT", "taskId required for observe"))?;
        Ok(crate::assistance::ObserveArgs {
            task_id: task_id.to_string(),
            filter: self.filter.clone(),
            lines: self.lines,
            before: self.before,
            after: self.after.map(|n| n as usize),
            stream: self.stream.clone(),
            cursor: self.cursor,
            max_chars: self.max_chars,
        })
    }
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

    #[test]
    fn accepts_observe_action_and_converts_cleanly() {
        let args: TaskArgs = serde_json::from_value(json!({
            "action": "observe",
            "taskId": "task-42",
            "filter": "error: .*",
            "lines": 15,
            "before": 3,
            "after": 5,
            "stream": "stderr",
            "cursor": 100,
            "maxChars": 8000
        }))
        .unwrap();
        assert_eq!(args.action, "observe");
        assert_eq!(args.task_id.as_deref(), Some("task-42"));
        let obs = args.into_observe().unwrap();
        assert_eq!(obs.task_id, "task-42");
        assert_eq!(obs.filter.as_deref(), Some("error: .*"));
        assert_eq!(obs.lines, Some(15));
        assert_eq!(obs.before, Some(3));
        assert_eq!(obs.after, Some(5));
        assert_eq!(obs.stream.as_deref(), Some("stderr"));
        assert_eq!(obs.cursor, Some(100));
        assert_eq!(obs.max_chars, Some(8000));
    }
}
