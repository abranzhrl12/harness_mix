use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{
    BatchArgs, BootstrapArgs, CheckpointArgs, HandoverArgs, Kind, MoveArgs, ReadArgs, ReplaceArgs,
    Request, SearchArgs, TasksAction, TasksArgs, WriteArgs,
};
use serde_json::json;

fn bootstrap(app: &NativeApplication) -> String {
    app.execute(Request::Bootstrap(BootstrapArgs {
        compact: false,
        session: None,
        include_rules: None,
    }))["data"]["sessionId"]
        .as_str()
        .unwrap()
        .into()
}

#[test]
fn session_alias_current_resolves_active_session_and_supports_full_workflow() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();

    // Calling with "current" before bootstrap should cleanly fail with SESSION error
    let early = app.execute(Request::Tasks(TasksArgs {
        session: "current".into(),
        action: None,
        phase: None,
        status: None,
        note: None,
        after: None,
        limit: None,
    }));
    assert_eq!(early["state"], "error");
    assert_eq!(early["diagnostics"][0]["code"], "SESSION");

    // Bootstrap establishes active session
    let id = bootstrap(&app);
    assert!(!id.is_empty());

    // Calling tasks with session = "current" succeeds
    let task_res = app.execute(Request::Tasks(TasksArgs {
        session: "current".into(),
        action: Some(TasksAction::Update),
        phase: Some("analysis".into()),
        status: Some("in_progress".into()),
        note: Some("working with current".into()),
        after: None,
        limit: None,
    }));
    assert_eq!(task_res["state"], "ok", "{task_res}");
    assert_eq!(task_res["data"]["persisted"], true);

    // Calling checkpoint with session = "current" succeeds
    let ckpt = app.execute(Request::Checkpoint(CheckpointArgs {
        session: "current".into(),
        goal: Some("test session alias".into()),
        decisions: None,
        pending: None,
        refresh_freshness: Some(false),
    }));
    assert_eq!(ckpt["state"], "ok", "{ckpt}");
    assert_eq!(
        ckpt["data"]["declaredByAgent"]["goal"],
        "test session alias"
    );

    // Handover with session = "current" succeeds
    let ho = app.execute(Request::Handover(HandoverArgs {
        session: "current".into(),
        kind: None,
        target_file: None,
        expected_hash: None,
    }));
    assert_eq!(ho["state"], "ok", "{ho}");
    assert_eq!(ho["data"]["written"], false);

    // Read and Search with session = "current" succeed
    std::fs::write(root.path().join("demo.txt"), "hello world\n").unwrap();
    let read_res = app.execute(Request::Read(ReadArgs {
        external_scope: None,
        path: "demo.txt".into(),
        start: 1,
        end: 1,
        compact: false,
        line_numbers: false,
        session: Some("current".into()),
    }));
    assert_eq!(read_res["state"], "ok", "{read_res}");
    assert_eq!(read_res["data"]["text"], "hello world");

    let search_res = app.execute(Request::Search(SearchArgs {
        override_project_exclusions: None,
        only_matching: None,
        ast: None,
        regex: None,
        case_sensitive: None,
        context_lines: None,
        external_scope: None,
        kind: Kind::Text,
        query: "hello".into(),
        min_lines: None,
        globs: None,
        scope: None,
        limit: None,
        session: Some("current".into()),
        cursor: None,
        offset: None,
    }));
    assert_eq!(search_res["state"], "ok", "{search_res}");

    // Replace with session = "current" succeeds
    let rep_res = app.execute(Request::Replace(ReplaceArgs {
        bom: None,
        path: "demo.txt".into(),
        session: Some("current".into()),
        hash: harness_domain::hash(b"hello world\n"),
        line: None,
        chunks: None,
        old_text: "world".into(),
        new_text: "universe".into(),
        allow_unvalidated_text: false,
        preview: false,
    }));
    assert_eq!(rep_res["state"], "ok", "{rep_res}");

    // Write with session = "current" succeeds
    let write_res = app.execute(Request::Write(WriteArgs {
        bom: None,
        path: "new_file.txt".into(),
        content: "created with current session".into(),
        create_parents: false,
        allow_unvalidated_text: false,
        expected_hash: None,
        overwrite: false,
        session: Some("current".into()),
    }));
    assert_eq!(write_res["state"], "ok", "{write_res}");

    // Move with session = "current" succeeds
    let move_res = app.execute(Request::Move(MoveArgs {
        session: Some("current".into()),
        from: "new_file.txt".into(),
        to: "renamed_file.txt".into(),
    }));
    assert_eq!(move_res["state"], "ok", "{move_res}");

    // Calling with "new" creates a second distinct session
    let new_res = app.execute(Request::Tasks(TasksArgs {
        session: "new".into(),
        action: None,
        phase: None,
        status: None,
        note: None,
        after: None,
        limit: None,
    }));
    assert_eq!(new_res["state"], "ok", "{new_res}");
    // The new session has default goal (not "test session alias")
    let new_ckpt = app.execute(Request::Checkpoint(CheckpointArgs {
        session: "current".into(),
        goal: None,
        decisions: None,
        pending: None,
        refresh_freshness: Some(false),
    }));
    assert_eq!(new_ckpt["state"], "ok", "{new_ckpt}");
    assert_ne!(
        new_ckpt["data"]["declaredByAgent"]["goal"],
        "test session alias"
    );

    // Resume the first session with "resume:<uuid>"
    let resume_res = app.execute(Request::Checkpoint(CheckpointArgs {
        session: format!("resume:{id}"),
        goal: None,
        decisions: None,
        pending: None,
        refresh_freshness: Some(false),
    }));
    assert_eq!(resume_res["state"], "ok", "{resume_res}");
    assert_eq!(
        resume_res["data"]["declaredByAgent"]["goal"],
        "test session alias"
    );

    // Now "current" points back to the resumed session!
    let current_again = app.execute(Request::Checkpoint(CheckpointArgs {
        session: "current".into(),
        goal: None,
        decisions: None,
        pending: None,
        refresh_freshness: Some(false),
    }));
    assert_eq!(current_again["state"], "ok", "{current_again}");
    assert_eq!(
        current_again["data"]["declaredByAgent"]["goal"],
        "test session alias"
    );
}

#[test]
fn session_alias_invalid_spec_fails_cleanly() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let _id = bootstrap(&app);

    // Invalid UUID / spec
    let bad = app.execute(Request::Tasks(TasksArgs {
        session: "not-a-valid-uuid-or-alias".into(),
        action: None,
        phase: None,
        status: None,
        note: None,
        after: None,
        limit: None,
    }));
    assert_eq!(bad["state"], "error");
    assert_eq!(bad["diagnostics"][0]["code"], "SESSION");

    // Invalid resume target
    let bad_resume = app.execute(Request::Tasks(TasksArgs {
        session: "resume:00000000-0000-0000-0000-000000000000".into(),
        action: None,
        phase: None,
        status: None,
        note: None,
        after: None,
        limit: None,
    }));
    assert_eq!(bad_resume["state"], "error");
    assert_eq!(bad_resume["diagnostics"][0]["code"], "SESSION");
}

#[test]
fn session_alias_works_with_batch_and_explore() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("main.rs"), "fn main() {}\n").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let _id = bootstrap(&app);

    // Batch with session = "current"
    let batch_args: BatchArgs = serde_json::from_value(json!({
        "session": "current",
        "jobs": [
            {
                "tool": "harness_read",
                "arguments": {
                    "path": "main.rs",
                    "start": 1,
                    "end": 1
                }
            }
        ]
    }))
    .unwrap();
    let batch_res = app.execute(Request::Batch(batch_args));
    assert_eq!(batch_res["state"], "ok", "{batch_res}");

    // Doctor with session = "current"
    let doctor_res = app.execute(Request::Doctor(
        serde_json::from_value(json!({
            "session": "current"
        }))
        .unwrap(),
    ));
    assert_eq!(doctor_res["state"], "ok", "{doctor_res}");

    // Explore with session = "current" is admitted and leases the session (returns INDEX without codegraph db, not SESSION)
    let explore_args = serde_json::from_value(json!({
        "query": "main",
        "session": "current"
    }))
    .unwrap();
    let explore_res = app.execute(Request::Explore(explore_args));
    assert_ne!(explore_res["diagnostics"][0]["code"], "SESSION");
}

#[test]
fn unified_session_tool_supports_tasks_checkpoint_status_and_handover() {
    use harness_domain::contracts::SessionArgs;

    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let id = bootstrap(&app);

    // 1. Session: action=checkpoint (mutating)
    let ckpt: SessionArgs = serde_json::from_value(json!({
        "action": "checkpoint",
        "session": id,
        "goal": "validate unified harness_session",
        "decisions": ["consolidate session tools into harness_session"],
        "pending": ["write integration test"]
    }))
    .unwrap();
    let res = app.execute(ckpt.into_request());
    assert_eq!(res["state"], "ok", "{res}");
    assert_eq!(
        res["data"]["declaredByAgent"]["goal"],
        "validate unified harness_session"
    );

    // 2. Session: action=tasks (update phase & status)
    let tasks: SessionArgs = serde_json::from_value(json!({
        "action": "tasks",
        "session": id,
        "phase": "implementation",
        "status": "in_progress",
        "note": "testing unified session dispatcher"
    }))
    .unwrap();
    let res = app.execute(tasks.into_request());
    assert_eq!(res["state"], "ok", "{res}");
    assert_eq!(res["data"]["persisted"], true);

    // 3. Session: action=tasks (timeline query)
    let timeline: SessionArgs = serde_json::from_value(json!({
        "action": "tasks",
        "session": id,
        "after": 0,
        "limit": 10
    }))
    .unwrap();
    let res = app.execute(timeline.into_request());
    assert_eq!(res["state"], "ok", "{res}");
    assert!(res["data"]["events"].as_array().is_some());

    // 4. Session: action=status (read-only live summary)
    let status: SessionArgs = serde_json::from_value(json!({
        "action": "status",
        "session": id
    }))
    .unwrap();
    let res = app.execute(status.into_request());
    assert_eq!(res["state"], "ok", "{res}");
    assert_eq!(res["data"]["sessionId"], id);
    assert_eq!(
        res["data"]["declaredByAgent"]["goal"],
        "validate unified harness_session"
    );

    // 5. Session: action=handover (generates summary report preview)
    let handover: SessionArgs = serde_json::from_value(json!({
        "action": "handover",
        "session": id,
        "kind": "summary"
    }))
    .unwrap();
    let res = app.execute(handover.into_request());
    assert_eq!(res["state"], "ok", "{res}");
    assert_eq!(res["data"]["kind"], "summary");
    assert!(res["data"]["preview"].as_str().is_some());
}
