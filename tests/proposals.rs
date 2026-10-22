#![cfg(windows)]
use agent_harness::adapters::{self, NativeApplication};
use harness_domain::{
    contracts::{ProposalArgs, Request},
    execution::OperationContext,
    hash,
    tool_contract::ToolProfile,
};
use serde_json::{Value, json};
use std::fs;

fn prepare(app: &NativeApplication, arguments: Value) -> Value {
    app.execute(Request::Proposal(
        serde_json::from_value(arguments).unwrap(),
    ))
}

#[test]
fn proposal_persists_exact_original_candidate_and_hashes_without_publishing() {
    let root = tempfile::tempdir().unwrap();
    let original = "\u{feff}uno\r\ndos\r\n";
    let candidate = "\u{feff}uno\r\ntres\r\n";
    fs::write(root.path().join("file.txt"), original).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = prepare(
        &app,
        json!({"action":"prepare","path":"FILE.txt","content":"uno\r\ntres\r\n","expectedHash":hash(original.as_bytes())}),
    );
    assert_eq!(result["state"], "ok", "{result}");
    let data = &result["data"];
    assert_eq!(data["stored"], true);
    assert_eq!(data["changed"], false);
    assert_eq!(data["published"], false);
    assert_eq!(data["status"], "pending");
    assert_eq!(data["path"], "file.txt");
    assert_eq!(data["baseHash"], hash(original.as_bytes()));
    assert_eq!(data["candidateHash"], hash(candidate.as_bytes()));
    assert!(
        data["patch"]
            .as_str()
            .unwrap()
            .contains("-dos\r\n+tres\r\n")
    );
    let id = data["proposalId"].as_str().unwrap();
    assert!(uuid::Uuid::parse_str(id).is_ok());
    let storage = root.path().join(data["storagePath"].as_str().unwrap());
    let saved: Value = serde_json::from_slice(&fs::read(&storage).unwrap()).unwrap();
    assert_eq!(saved["format"], "agent-harness-proposal-v1");
    assert_eq!(saved["original"], original);
    assert_eq!(saved["candidate"], candidate);
    assert_eq!(saved["proposalId"], id);
    assert_eq!(
        fs::read(root.path().join("file.txt")).unwrap(),
        original.as_bytes()
    );
    assert!(!root.path().join(".codegraph").exists());
    assert_eq!(
        fs::read(root.path().join(".agents/harness/proposals/.gitignore")).unwrap(),
        b"*\n"
    );
    drop(app);
    // Restarting must never apply a pending proposal or undo later user edits.
    fs::write(root.path().join("file.txt"), "later user edit").unwrap();
    let _restarted = adapters::application(root.path()).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(storage).unwrap()).unwrap(),
        saved
    );
    assert_eq!(
        fs::read_to_string(root.path().join("file.txt")).unwrap(),
        "later user edit"
    );
}

#[test]
fn stale_invalid_and_noop_requests_do_not_create_proposal_storage() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.json"), "{\"value\":1}").unwrap();
    let app = adapters::application(root.path()).unwrap();
    for (arguments, code) in [
        (
            json!({"action":"prepare","path":"a.json","content":"{}","expectedHash":hash(b"older")}),
            "STALE",
        ),
        (
            json!({"action":"prepare","path":"a.json","content":"{}","expectedHash":"invalid"}),
            "ARGUMENT",
        ),
        (
            json!({"action":"prepare","path":"a.json","content":"{"}),
            "SYNTAX_ERROR",
        ),
        (
            json!({"action":"prepare","path":"a.json","content":"\0"}),
            "ENCODING",
        ),
        (
            json!({"action":"prepare","path":"a.json","content":"x".repeat(2 * 1024 * 1024 + 1)}),
            "SIZE",
        ),
    ] {
        let result = prepare(&app, arguments);
        assert_eq!(result["diagnostics"][0]["code"], code, "{result}");
        assert!(!root.path().join(".agents/harness/proposals").exists());
    }
    let result = prepare(
        &app,
        json!({"action":"prepare","path":"a.json","content":"{\"value\":1}"}),
    );
    assert_eq!(result["data"]["stored"], false);
    assert_eq!(result["data"]["status"], "unchanged");
    assert!(!root.path().join(".agents/harness/proposals").exists());
    assert_eq!(
        fs::read_to_string(root.path().join("a.json")).unwrap(),
        "{\"value\":1}"
    );
}

#[test]
fn proposal_is_opt_in_and_preserves_current_preview_replace_write_and_delete() {
    use harness_domain::contracts::{DeleteArgs, ReplaceArgs, WriteArgs};
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file.txt"), "old").unwrap();
    let app = adapters::application(root.path()).unwrap();
    assert!(!root.path().join(".agents/harness/proposals").exists());
    let preview: ReplaceArgs = serde_json::from_value(
        json!({"path":"file.txt","oldText":"old","newText":"preview","preview":true}),
    )
    .unwrap();
    assert_eq!(app.execute(Request::Replace(preview))["state"], "ok");
    assert_eq!(
        fs::read_to_string(root.path().join("file.txt")).unwrap(),
        "old"
    );
    let edit: ReplaceArgs =
        serde_json::from_value(json!({"path":"file.txt","oldText":"old","newText":"edited"}))
            .unwrap();
    assert_eq!(app.execute(Request::Replace(edit))["state"], "ok");
    assert_eq!(
        fs::read_to_string(root.path().join("file.txt")).unwrap(),
        "edited"
    );
    let write: WriteArgs =
        serde_json::from_value(json!({"path":"created.txt","content":"new"})).unwrap();
    assert_eq!(app.execute(Request::Write(write))["state"], "ok");
    let delete: DeleteArgs = serde_json::from_value(json!({"path":"created.txt"})).unwrap();
    assert_eq!(app.execute(Request::Delete(delete))["state"], "ok");
    assert!(!root.path().join(".agents/harness/proposals").exists());
    let result = prepare(
        &app,
        json!({"action":"prepare","path":"file.txt","content":"proposed"}),
    );
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(
        fs::read_to_string(root.path().join("file.txt")).unwrap(),
        "edited"
    );
}

#[test]
fn profile_cancellation_and_protected_sources_do_not_store_proposals() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file.txt"), "keep").unwrap();
    fs::write(root.path().join("AGENTS.md"), "instructions").unwrap();
    let app = adapters::application(root.path()).unwrap();
    for path in [
        "AGENTS.md",
        "../file.txt",
        "C:/file.txt",
        "missing.txt",
        ".",
        ".env",
    ] {
        assert_eq!(
            prepare(
                &app,
                json!({"action":"prepare","path":path,"content":"new"})
            )["state"],
            "error",
            "{path}"
        );
    }
    let context = OperationContext::default();
    context.cancel();
    let args: ProposalArgs =
        serde_json::from_value(json!({"action":"prepare","path":"file.txt","content":"new"}))
            .unwrap();
    assert_eq!(
        app.execute_with_context(Request::Proposal(args), context)["state"],
        "cancelled"
    );
    let app = app.with_tool_profile(ToolProfile::Investigate);
    assert_eq!(
        prepare(
            &app,
            json!({"action":"prepare","path":"file.txt","content":"new"})
        )["diagnostics"][0]["code"],
        "TOOL_PROFILE"
    );
    assert!(!root.path().join(".agents/harness/proposals").exists());
    assert_eq!(
        fs::read_to_string(root.path().join("file.txt")).unwrap(),
        "keep"
    );
    assert!(ToolProfile::Edit.allows("harness_proposal"));
}

#[test]
fn storage_junction_is_rejected_without_writing_outside_project() {
    use std::{os::windows::process::CommandExt, process::Command};
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file.txt"), "keep").unwrap();
    fs::create_dir_all(root.path().join(".agents/harness")).unwrap();
    let junction = root
        .path()
        .join(".agents")
        .join("harness")
        .join("proposals");
    let output = Command::new("cmd")
        .creation_flags(0x0800_0000)
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let app = adapters::application(root.path()).unwrap();
    let result = prepare(
        &app,
        json!({"action":"prepare","path":"file.txt","content":"new"}),
    );
    assert_eq!(
        result["diagnostics"][0]["code"], "PROPOSAL_PATH",
        "{result}"
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    assert_eq!(
        fs::read_to_string(root.path().join("file.txt")).unwrap(),
        "keep"
    );
    fs::remove_dir(junction).unwrap();
}

#[test]
fn unexpected_storage_policy_is_not_overwritten() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file.txt"), "keep").unwrap();
    let directory = root.path().join(".agents/harness/proposals");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join(".gitignore"), "user policy\n").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = prepare(
        &app,
        json!({"action":"prepare","path":"file.txt","content":"new"}),
    );
    assert_eq!(
        result["diagnostics"][0]["code"], "PROPOSAL_PATH",
        "{result}"
    );
    assert_eq!(
        fs::read_to_string(directory.join(".gitignore")).unwrap(),
        "user policy\n"
    );
    assert_eq!(fs::read_dir(directory).unwrap().count(), 1);
    assert_eq!(
        fs::read_to_string(root.path().join("file.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn unknown_actions_are_rejected_and_large_diff_is_retrievable() {
    for action in ["accept", "reject", "abort", "apply"] {
        assert!(
            serde_json::from_value::<ProposalArgs>(
                json!({"action":action,"path":"a.txt","content":"new"})
            )
            .is_err()
        );
    }
    let root = tempfile::tempdir().unwrap();
    let original = "old line\n".repeat(1500);
    fs::write(root.path().join("file.txt"), &original).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = prepare(
        &app,
        json!({"action":"prepare","path":"file.txt","content":"new line\n".repeat(1500)}),
    );
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["omitted"]["truncated"], true);
    assert!(result["detail"]["ref"].is_string());
    let detail = app.execute(Request::Detail(
        serde_json::from_value(json!({
            "ref":result["detail"]["ref"],"offset":0,"maxChars":16000
        }))
        .unwrap(),
    ));
    assert_eq!(detail["state"], "ok", "{detail}");
    assert!(
        detail["data"]["text"]
            .as_str()
            .unwrap()
            .contains("-old line")
    );
    assert_eq!(
        fs::read_to_string(root.path().join("file.txt")).unwrap(),
        original
    );
}

#[test]
fn preparation_does_not_add_modified_file_observations_to_session() {
    use harness_domain::contracts::{BootstrapArgs, CheckpointArgs};
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file.txt"), "old").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let boot: BootstrapArgs = serde_json::from_value(json!({"compact":true})).unwrap();
    let result = app.execute(Request::Bootstrap(boot));
    let session = result["data"]["session"].as_str().unwrap();
    let a: CheckpointArgs = serde_json::from_value(json!({"session":session})).unwrap();
    let before = app.execute(Request::Checkpoint(a));
    let proposal = prepare(
        &app,
        json!({"action":"prepare","path":"file.txt","content":"new","session":session}),
    );
    assert_eq!(proposal["state"], "ok", "{proposal}");
    let a: CheckpointArgs = serde_json::from_value(json!({"session":session})).unwrap();
    let after = app.execute(Request::Checkpoint(a));
    assert_eq!(
        before["data"]["observedByHarness"],
        after["data"]["observedByHarness"]
    );
}
