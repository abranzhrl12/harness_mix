//! Strict persisted metadata format. Never deserialize arbitrary messages into logs.
use crate::{Failure, execution::valid_request_id};
use serde_json::Value;
pub const FORMAT: &str = "agent-harness-native-logs-v1";
pub const MAX_BYTES: usize = 256 * 1024;
pub const MAX_EVENTS: usize = 512;
const INVALID: Failure = (
    "LOG_FORMAT",
    "Invalid native log archive; no content returned or overwritten",
);
pub fn validate(value: &Value) -> Result<(), Failure> {
    let object = value.as_object().ok_or(INVALID)?;
    if object.len() != 4 || value["format"] != FORMAT || value["discarded"].as_u64().is_none() {
        return Err(INVALID);
    }
    let last = value["lastSequence"].as_u64().ok_or(INVALID)?;
    if last > 9007199254740991 {
        return Err(INVALID);
    }
    let events = value["events"].as_array().ok_or(INVALID)?;
    if events.len() > MAX_EVENTS {
        return Err(INVALID);
    }
    let discarded = value["discarded"].as_u64().ok_or(INVALID)?;
    if discarded.checked_add(events.len() as u64) != Some(last) {
        return Err(INVALID);
    }
    let mut previous = discarded;
    for event in events {
        if event.as_object().is_none_or(|o| o.len() != 10) {
            return Err(INVALID);
        }
        let sequence = event["sequence"].as_u64().ok_or(INVALID)?;
        if sequence != previous + 1 || sequence > last {
            return Err(INVALID);
        }
        previous = sequence;
        if !event["requestId"].as_str().is_some_and(valid_request_id)
            || event["uptimeMs"].as_u64().is_none()
            || !event["tool"].as_str().is_some_and(|s| {
                [
                    "harness_bootstrap",
                    "harness_explore",
                    "harness_read",
                    "harness_detail",
                    "harness_search",
                    "harness_verify",
                    "harness_replace",
                    "harness_move",
                    "harness_outline",
                    "harness_read_symbol",
                    "harness_tasks",
                    "harness_checkpoint",
                    "harness_git_status",
                    "harness_git",
                    "harness_git_diff",
                    "harness_doctor",
                    "harness_batch",
                    "harness_logs",
                    "harness_config",
                    "harness_external_scope",
                ]
                .contains(&s)
            })
            || !event["kind"]
                .as_str()
                .is_some_and(|s| ["completed", "rejected", "abandoned"].contains(&s))
            || (!event["state"].is_null()
                && !event["state"]
                    .as_str()
                    .is_some_and(|s| ["ok", "error", "cancelled", "timeout"].contains(&s)))
            || !event["level"]
                .as_str()
                .is_some_and(|s| ["info", "warn", "error"].contains(&s))
            || (!event["durationMicros"].is_null() && event["durationMicros"].as_u64().is_none())
            || event["slow"].as_bool().is_none()
        {
            return Err(INVALID);
        }
        if !event["code"].is_null()
            && !event["code"].as_str().is_some_and(|s| {
                [
                    "PATH",
                    "MOVE_EXISTS",
                    "MOVE_IO",
                    "MOVE_POSTCHECK",
                    "MOVE_INDEX",
                    "ARGUMENT",
                    "QUEUE_FULL",
                    "CANCELLED",
                    "TIMEOUT",
                    "ENCODING",
                    "SYNTAX_ERROR",
                    "PARSER_UNAVAILABLE",
                    "STALE_HASH",
                    "STALE_SYMBOL",
                    "AMBIGUOUS_SYMBOL",
                    "VERIFY_FAILED",
                    "BATCH_PARTIAL",
                    "UNSUPPORTED",
                    "GIT_REPO",
                    "GIT_READ",
                    "GIT_SIZE",
                    "INDEX",
                    "INDEX_STALE",
                    "INDEX_UNCHECKED",
                    "GRAPH_CONTRACT",
                    "GRAPH_LIMIT",
                    "SESSION_BUSY",
                    "INTERNAL",
                    "TOOL_ERROR",
                    "CONFIG_XML",
                    "CONFIG_JSON",
                    "SECTION",
                    "AMBIGUOUS_SECTION",
                    "AUTHORIZATION",
                    "EXTERNAL_SCOPE",
                    "EXTERNAL_SCOPE_LIMIT",
                    "STALE_SCOPE",
                ]
                .contains(&s)
            })
        {
            return Err(INVALID);
        }
        for key in [
            "sequence",
            "requestId",
            "uptimeMs",
            "tool",
            "kind",
            "state",
            "level",
            "code",
            "durationMicros",
            "slow",
        ] {
            if event.get(key).is_none() {
                return Err(INVALID);
            }
        }
    }
    if previous != last {
        return Err(INVALID);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> Value {
        json!({"format":FORMAT,"lastSequence":1,"discarded":0,"events":[{
            "sequence":1,"requestId":crate::execution::new_request_id(),"uptimeMs":0,"tool":"harness_read","kind":"completed","state":"ok","level":"info","code":null,"durationMicros":10,"slow":false
        }]})
    }
    #[test]
    fn archive_rejects_unknown_content_and_noncanonical_identifiers() {
        let valid = fixture();
        assert!(validate(&valid).is_ok());
        let mut injected = valid.clone();
        injected["events"][0]["message"] = json!("private data");
        assert!(validate(&injected).is_err());
        let mut injected = valid.clone();
        injected["events"][0]["tool"] = json!("unrecognized private data");
        assert!(validate(&injected).is_err());
        let mut injected = valid.clone();
        injected["events"][0]["requestId"] = json!("not-an-id");
        assert!(validate(&injected).is_err());
        let mut injected = valid;
        injected["events"][0]["code"] = json!("arbitrary diagnostic text");
        assert!(validate(&injected).is_err());
    }
    #[test]
    fn archive_sequences_and_cardinality_are_bounded() {
        let mut value = fixture();
        value["lastSequence"] = json!(2);
        assert!(validate(&value).is_err());
        let event = value["events"][0].clone();
        value["events"] = json!(vec![event; MAX_EVENTS + 1]);
        assert!(validate(&value).is_err());
        assert!(
            validate(&json!({"format":FORMAT,"lastSequence":0,"discarded":0,"events":[]})).is_ok()
        );
    }
}
