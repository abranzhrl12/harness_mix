use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{
    BootstrapArgs, CheckpointArgs, ReplaceArgs, Request, TasksAction, TasksArgs,
};
use serde_json::Value;

fn bootstrap(app: &NativeApplication) -> String {
    app.execute(Request::Bootstrap(BootstrapArgs {
        session: None,
        include_rules: None,
    }))["data"]["sessionId"]
        .as_str()
        .unwrap()
        .into()
}
fn task(
    app: &NativeApplication,
    session: &str,
    phase: Option<&str>,
    status: Option<&str>,
    note: Option<&str>,
) -> Value {
    app.execute(Request::Tasks(TasksArgs {
        session: session.into(),
        action: phase.map(|_| TasksAction::Update),
        phase: phase.map(Into::into),
        status: status.map(Into::into),
        note: note.map(Into::into),
        after: None,
        limit: None,
    }))
}
fn checkpoint(app: &NativeApplication, session: &str, goal: Option<&str>, refresh: bool) -> Value {
    app.execute(Request::Checkpoint(CheckpointArgs {
        session: session.into(),
        goal: goal.map(Into::into),
        decisions: None,
        pending: None,
        refresh_freshness: Some(refresh),
    }))
}

#[test]
fn phase_updates_checkpoint_and_history_survive_restart_without_certifying_freshness() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let id = bootstrap(&app);
    assert_eq!(task(&app, &id, None, None, None)["data"]["latestEvent"], 1);
    let phase = task(&app, &id, Some("analysis"), Some("in_progress"), None);
    assert_eq!(phase["state"], "ok", "{phase}");
    assert_eq!(phase["data"]["persisted"], true);
    assert_eq!(
        task(&app, &id, Some("implementation"), Some("in_progress"), None)["diagnostics"][0]["code"],
        "TASK"
    );
    let declared = checkpoint(&app, &id, Some("migrar 🙂"), false);
    assert_eq!(declared["data"]["persisted"], true, "{declared}");
    assert_eq!(
        checkpoint(&app, &id, Some("must not apply"), true)["diagnostics"][0]["code"],
        "UNSUPPORTED"
    );
    assert_eq!(
        checkpoint(&app, &id, None, false)["data"]["declaredByAgent"]["goal"],
        "migrar 🙂"
    );
    drop(app);
    let app = adapters::application(root.path()).unwrap();
    let restored = checkpoint(&app, &id, None, false);
    assert_eq!(restored["state"], "ok", "{restored}");
    assert_eq!(restored["data"]["restored"], true);
    assert_eq!(restored["data"]["tasks"]["active"], "analysis");
    assert_eq!(restored["data"]["declaredByAgent"]["goal"], "migrar 🙂");
    assert_eq!(restored["data"]["freshness"]["available"], false);
    assert!(restored["detail"]["ref"].is_string());
}

#[cfg(windows)]
#[test]
fn edits_are_observed_then_restored_as_history_not_new_current_facts() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file.txt"), "old").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let id = bootstrap(&app);
    let edit = app.execute(Request::Replace(ReplaceArgs {
        session: Some(id.clone()),
        path: "file.txt".into(),
        hash: harness_domain::hash(b"old"),
        old_text: "old".into(),
        new_text: "new".into(),
    }));
    assert_eq!(edit["state"], "ok", "{edit}");
    assert!(edit["data"].get("sessionWarning").is_none(), "{edit}");
    let current = checkpoint(&app, &id, None, false);
    assert_eq!(
        current["data"]["observedByHarness"]["modifiedFiles"][0]["path"],
        "file.txt"
    );
    assert!(current["data"]["historical"].is_null());
    let timeline = app.execute(Request::Tasks(TasksArgs {
        session: id.clone(),
        action: Some(TasksAction::Timeline),
        phase: None,
        status: None,
        note: None,
        after: Some(0),
        limit: Some(20),
    }));
    assert_eq!(timeline["data"]["events"][1]["type"], "edit_applied");
    assert_eq!(timeline["data"]["events"][1]["source"], "harness-observed");
    drop(app);
    let app = adapters::application(root.path()).unwrap();
    let restored = checkpoint(&app, &id, None, false);
    assert_eq!(
        restored["data"]["historical"]["modifiedFiles"][0]["hash"],
        harness_domain::hash(b"new")
    );
    assert!(
        restored["data"]["observedByHarness"]["modifiedFiles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn successful_verification_is_recorded_without_claiming_current_code_freshness() {
    use harness_domain::contracts::{VerifyAction, VerifyArgs};
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname='workflow-fixture'\nversion='0.1.0'\nedition='2024'\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(root.path().join("src/lib.rs"), "pub fn sample() {}\n").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let id = bootstrap(&app);
    let result = app.execute(Request::Verify(VerifyArgs {
        language: "rust".into(),
        scope: None,
        action: Some(VerifyAction::Build),
        session: Some(id.clone()),
    }));
    assert_eq!(result["state"], "ok", "{result}");
    assert!(result["data"].get("sessionWarning").is_none());
    let current = checkpoint(&app, &id, None, false);
    assert_eq!(
        current["data"]["observedByHarness"]["checks"][0]["ok"],
        true
    );
    assert!(current["data"]["observedByHarness"]["checks"][0]["current"].is_null());
    assert!(
        current["data"]["observedByHarness"]["checks"][0]["time"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    drop(app);
    let app = adapters::application(root.path()).unwrap();
    let restored = checkpoint(&app, &id, None, false);
    assert_eq!(restored["data"]["historical"]["checks"][0]["ok"], true);
    assert!(
        restored["data"]["observedByHarness"]["checks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(restored["data"]["freshness"]["available"], false);
}

#[test]
fn invalid_updates_do_not_poison_a_session_and_persistence_failure_is_explicit() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let id = bootstrap(&app);
    assert_eq!(
        checkpoint(&app, &id, Some(&"🙂".repeat(501)), false)["diagnostics"][0]["code"],
        "CHECKPOINT"
    );
    assert_eq!(
        checkpoint(&app, &id, None, false)["data"]["declaredByAgent"]["goal"],
        ""
    );
    // Replace only the test's session file with a directory to force a safe save failure.
    let file = root
        .path()
        .join(format!(".agents/harness/.state/{id}.json"));
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    let update = task(
        &app,
        &id,
        Some("analysis"),
        Some("completed"),
        Some("memory only"),
    );
    assert_eq!(update["state"], "ok", "{update}");
    assert_eq!(update["data"]["persisted"], false);
    assert_eq!(update["data"]["persistenceWarning"]["memoryUpdated"], true);
    assert_eq!(task(&app, &id, None, None, None)["data"]["completed"], 1);
}
