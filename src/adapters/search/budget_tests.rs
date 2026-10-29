use super::*;
use serde_json::json;

#[test]
fn display_limit_does_not_mean_scan_was_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sample.txt"), "hit\n".repeat(650)).unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args = serde_json::from_value(json!({"query":"hit", "limit":40})).unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["omitted"], 610);
    assert_eq!(result["complete"], true);
    assert_eq!(result["presentationTruncated"], true);
    assert_eq!(result["contentTruncated"], false);
    assert_eq!(result["scanIncompleteReasons"], json!([]));
    assert_eq!(result["diagnostics"], json!([]));
    assert_eq!(result["limits"]["scanMs"], 15000);
}

#[test]
fn stored_match_budget_reports_actual_reason() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sample.txt"), "hit\n".repeat(10001)).unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args = serde_json::from_value(json!({"query":"hit"})).unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["complete"], false);
    assert_eq!(result["limited"], true);
    assert!(
        result["scanIncompleteReasons"]
            .as_array()
            .unwrap()
            .contains(&json!("match_count_budget"))
    );
    assert_eq!(result["_detail"]["complete"], false);
}

#[test]
fn glob_filtered_files_do_not_consume_scan_budget() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..10001 {
        std::fs::write(dir.path().join(format!("a{i:05}.txt")), "ignored").unwrap();
    }
    std::fs::write(dir.path().join("z_match.rs"), "needle\n").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args = serde_json::from_value(json!({"query":"needle", "globs":["*.rs"]})).unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["complete"], true);
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
}

#[test]
fn exact_match_budget_is_complete_when_no_more_matches_exist() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sample.txt"), "needle\n".repeat(10000)).unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args = serde_json::from_value(json!({"query":"needle"})).unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["complete"], true);
}

#[test]
fn oversized_text_file_reports_size_limit() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("large.txt"), "x".repeat(2 * 1024 * 1024 + 1)).unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args = serde_json::from_value(json!({"query":"needle"})).unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["complete"], false);
    assert!(result["scanIncompleteReasons"].as_array().unwrap().contains(&json!("file_size_limit")));
}

#[test]
fn long_line_preview_retains_full_source_in_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let source = format!("hit{}\n", "x".repeat(800));
    std::fs::write(dir.path().join("sample.txt"), &source).unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args = serde_json::from_value(json!({"query":"hit"})).unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["contentTruncated"], true);
    assert_eq!(result["presentationTruncated"], false);
    assert_eq!(result["complete"], true);
    assert_eq!(result["_detail"]["matches"][0]["text"], source);
}

#[test]
fn scan_no_longer_times_out_at_the_old_five_second_cutoff() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args = serde_json::from_value(json!({"kind":"file_size"})).unwrap();
    let context = OperationContext::default();
    let plan = plan::build(&project, &args).unwrap();
    let mut outcome = scan::execute(&project, &args, &context, plan, None).unwrap();
    outcome.started = std::time::Instant::now() - std::time::Duration::from_secs(6);
    let outcome = scan::finalize(outcome, &args, &project, &context);
    assert!(outcome.complete);
    assert!(!outcome.timed_out);
}
