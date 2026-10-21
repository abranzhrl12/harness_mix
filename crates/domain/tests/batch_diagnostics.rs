use harness_domain::contracts::BatchJob;
use serde_json::json;

#[test]
fn batch_decode_does_not_echo_values_disguised_as_expected_messages() {
    let job: BatchJob = serde_json::from_value(json!({
        "tool":"harness_sql",
        "arguments":{"action":"query", "sql":"SELECT 1",
            "engine":"invalid, expected DO_NOT_ECHO"}
    }))
    .unwrap();
    let error = job
        .into_request_detailed(None)
        .err()
        .expect("invalid arguments must fail");
    assert!(!error.reason.contains("DO_NOT_ECHO"));
    assert!(!format!("{error:?}").contains("DO_NOT_ECHO"));
}

#[test]
fn unknown_batch_fields_have_bounded_names_without_echoing_their_values() {
    let mut arguments = serde_json::Map::new();
    arguments.insert("x".repeat(500), json!("DO_NOT_ECHO"));
    let job: BatchJob = serde_json::from_value(json!({
        "tool":"harness_read", "arguments":arguments
    }))
    .unwrap();
    let error = job
        .into_request_detailed(None)
        .err()
        .expect("unknown fields must fail");
    assert_eq!(error.field.as_ref().unwrap().chars().count(), 80);
    assert!(!format!("{error:?}").contains("DO_NOT_ECHO"));
}
