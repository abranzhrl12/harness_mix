//! Integration tests for native harness_handover.

use agent_harness::adapters;
use harness_domain::contracts::{BootstrapArgs, HandoverArgs, HandoverKind, Request};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_handover_preview_summary_and_file_export() {
    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();

    // 1. Bootstrap session
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        compact: false,
        session: None,
        include_rules: None,
    }));
    assert_eq!(boot["state"], "ok");
    let session_id = boot["data"]["sessionId"].as_str().unwrap().to_string();

    // 2. Handover preview (without target_file)
    let handover_preview = app.execute(Request::Handover(HandoverArgs {
        session: session_id.clone(),
        kind: Some(HandoverKind::Summary),
        target_file: None,
        expected_hash: None,
    }));
    assert_eq!(handover_preview["state"], "ok", "{handover_preview}");
    let data = &handover_preview["data"];
    assert_eq!(data["kind"], "summary");
    assert_eq!(data["written"], false);
    let preview_str = data["preview"].as_str().unwrap();
    assert!(preview_str.contains("# Resumen de sesión"));
    assert!(preview_str.contains("Alcance: solo operaciones registradas"));

    // 3. Handover export to target_file
    let target_file = "HANDOVER_REPORT.md";
    let handover_export = app.execute(Request::Handover(HandoverArgs {
        session: session_id.clone(),
        kind: Some(HandoverKind::Summary),
        target_file: Some(target_file.to_string()),
        expected_hash: None,
    }));
    assert_eq!(handover_export["state"], "ok", "{handover_export}");
    let export_data = &handover_export["data"];
    assert_eq!(export_data["written"], true);
    let file_path = dir.path().join(target_file);
    assert!(file_path.exists());
    let content = fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("# Resumen de sesión"));
    assert_eq!(
        export_data["hash"],
        harness_domain::hash(content.as_bytes())
    );

    // 4. Handover overwrite failure if expected_hash mismatch
    let bad_overwrite = app.execute(Request::Handover(HandoverArgs {
        session: session_id,
        kind: Some(HandoverKind::Summary),
        target_file: Some(target_file.to_string()),
        expected_hash: Some(
            "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        ),
    }));
    assert_eq!(bad_overwrite["state"], "error");
    assert_eq!(bad_overwrite["diagnostics"][0]["code"], "STALE");
}
