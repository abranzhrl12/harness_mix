use super::{FILE, operations::ignore, policy};
use crate::adapters::project::Project;
use harness_domain::{
    contracts::{IgnoreAction, IgnoreArgs},
    execution::OperationContext,
};
use serde_json::json;
use std::fs;

fn args(action: IgnoreAction, patterns: &[&str]) -> IgnoreArgs {
    IgnoreArgs {
        action,
        scope: None,
        patterns: Some(
            patterns
                .iter()
                .map(|pattern| (*pattern).to_owned())
                .collect(),
        ),
        reason: None,
        session: None,
    }
}

#[test]
fn add_is_idempotent_and_preserves_existing_entries() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(FILE), "# keep\n*.log\n").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let mut request = args(IgnoreAction::Add, &["dist/", "*.log"]);
    request.reason = Some("generated output".into());
    let result = ignore(&project, &request, &OperationContext::default()).unwrap();
    assert_eq!(result["added"], json!(["dist/"]));
    assert_eq!(result["patterns"], 2);
    let text = fs::read_to_string(dir.path().join(FILE)).unwrap();
    assert!(text.contains("# keep\n*.log\n"));
    assert!(text.contains("# generated output\ndist/\n"));
    let again = ignore(&project, &request, &OperationContext::default()).unwrap();
    assert_eq!(again["unchanged"], true);
}

#[test]
fn remove_and_init_are_explicit_and_safe() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::new(dir.path()).unwrap();
    let init = ignore(
        &project,
        &args(IgnoreAction::Init, &[]),
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(init["created"], true);
    assert!(
        fs::read_to_string(dir.path().join(FILE))
            .unwrap()
            .starts_with("# Harness")
    );
    ignore(
        &project,
        &args(IgnoreAction::Add, &["build/"]),
        &OperationContext::default(),
    )
    .unwrap();
    let removed = ignore(
        &project,
        &args(IgnoreAction::Remove, &["build/"]),
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(removed["removed"], json!(["build/"]));
    assert_eq!(removed["patterns"], 0);
}

#[test]
fn unsafe_patterns_are_rejected() {
    for pattern in [
        "*",
        "../outside",
        "/absolute",
        "!.env",
        "!secret.txt",
        ".git/",
    ] {
        let error = policy::requested_patterns(&args(IgnoreAction::Add, &[pattern])).unwrap_err();
        assert!(
            error.0 == "ARGUMENT" || error.0 == "PATH",
            "{pattern}: {error:?}"
        );
    }
}

#[test]
fn pure_add_and_remove_preserve_policy_text_rules() {
    let source = "# keep\n*.log\n";
    let add = policy::add_patterns(
        source,
        &["dist/".to_owned(), "*.log".to_owned()],
        Some("generated output"),
    )
    .unwrap();
    assert_eq!(add.added, ["dist/"]);
    assert_eq!(add.text, "# keep\n*.log\n# generated output\ndist/\n");
    let remove = policy::remove_patterns(&add.text, &["dist/".to_owned()]);
    assert_eq!(remove.removed, ["dist/"]);
    assert_eq!(remove.text, "# keep\n*.log\n# generated output\n");
}

#[test]
fn pure_add_noop_preserves_original_text_and_ignores_reason() {
    let source = "# keep\r\nbuild/\r\n";
    let outcome =
        policy::add_patterns(source, &["build/".to_owned()], Some("should not be added")).unwrap();

    assert!(outcome.added.is_empty());
    assert_eq!(outcome.text, source);
}

#[test]
fn pure_remove_normalizes_crlf_and_keeps_comments() {
    let source = "# keep\r\nremove/\r\n# explain this rule\r\nstay/\r\n";
    let outcome = policy::remove_patterns(source, &["remove/".to_owned()]);

    assert_eq!(outcome.removed, ["remove/"]);
    assert_eq!(outcome.text, "# keep\n# explain this rule\nstay/\n");
}

#[test]
fn pure_add_deduplicates_requested_patterns_before_appending() {
    let requested =
        policy::requested_patterns(&args(IgnoreAction::Add, &["build/", "build/", "cache/"]))
            .unwrap();
    let outcome = policy::add_patterns("# keep\n", &requested, None).unwrap();

    assert_eq!(requested, ["build/", "cache/"]);
    assert_eq!(outcome.added, ["build/", "cache/"]);
    assert_eq!(outcome.text, "# keep\nbuild/\ncache/\n");
}
