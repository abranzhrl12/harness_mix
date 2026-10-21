use harness_domain::contracts::{BatchArgs, GitArgs, VerifyArgs};
use serde_json::json;

#[test]
fn public_arguments_accept_the_new_workflow_options() {
    let verify: VerifyArgs = serde_json::from_value(json!({
        "language":"dotnet","scope":"FAST.csproj","action":"build",
        "engine":"auto","warnings":"changed-files"
    }))
    .unwrap();
    assert_eq!(verify.engine.as_deref(), Some("auto"));
    assert_eq!(verify.warnings.as_deref(), Some("changed-files"));
    let git: GitArgs = serde_json::from_value(json!({
        "action":"diff","path":".","comparison":"auto"
    }))
    .unwrap();
    assert_eq!(git.into_request().arguments()["comparison"], "auto");
    let batch: BatchArgs = serde_json::from_value(json!({
        "jobs":[{"tool":"harness_search","args":{"kind":"files","scope":"."}}],
        "maxConcurrent":4
    }))
    .unwrap();
    assert_eq!(batch.max_concurrent, Some(4));
    assert_eq!(
        batch
            .jobs
            .into_iter()
            .next()
            .unwrap()
            .into_request(None)
            .unwrap()
            .arguments()["scope"],
        "."
    );
}

#[cfg(feature = "schema")]
#[test]
fn schemas_advertise_the_parameters_accepted_by_the_runtime() {
    let verify = serde_json::to_value(schemars::schema_for!(VerifyArgs)).unwrap();
    assert!(
        verify["properties"]["engine"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("auto"))
    );
    assert!(
        verify["properties"]["warnings"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("changed-files"))
    );
    let batch = serde_json::to_value(schemars::schema_for!(BatchArgs)).unwrap();
    assert_eq!(batch["properties"]["maxConcurrent"]["minimum"], 2);
    assert_eq!(batch["properties"]["maxConcurrent"]["maximum"], 4);
    let job = &batch["$defs"]["BatchJob"];
    assert!(job["properties"]["arguments"].is_object());
    assert!(job["properties"]["args"].is_object());
    let git = serde_json::to_value(schemars::schema_for!(GitArgs)).unwrap();
    assert!(git.to_string().contains("HEAD:worktree"));
}
