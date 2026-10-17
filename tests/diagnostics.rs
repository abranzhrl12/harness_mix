//! Integration tests for native harness_diagnostics.

use agent_harness::adapters;
use harness_domain::contracts::{BootstrapArgs, DiagnosticsArgs, Request};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_diagnostics_syntax_checks_rust_ts_js_json() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    fs::write(root.join("valid.rs"), "fn main() {}\n").unwrap();
    fs::write(root.join("valid.js"), "const x = 1;\n").unwrap();
    fs::write(root.join("valid.ts"), "const y: number = 42;\n").unwrap();
    fs::write(root.join("valid.json"), "{\"status\": \"ok\"}\n").unwrap();
    fs::write(root.join("readme.md"), "# Heading\n").unwrap();

    let app = adapters::application(root).unwrap();
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        session: None,
        include_rules: None,
    }));
    let session = boot["data"]["sessionId"].as_str().unwrap().to_string();

    let res = app.execute(Request::Diagnostics(DiagnosticsArgs {
        session: Some(session),
        files: Some(vec![
            "valid.rs".into(),
            "valid.js".into(),
            "valid.ts".into(),
            "valid.json".into(),
            "readme.md".into(),
        ]),
        max_files: Some(10),
        build_scope: None,
    }));

    assert_eq!(res["state"], "ok");
    assert_eq!(res["data"]["pass"], false); // Because readme.md is skipped, so not complete
    assert_eq!(res["data"]["complete"], false);
    assert_eq!(res["data"]["status"], "incomplete");
    assert_eq!(res["data"]["errors"], 0);

    let files = res["data"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 5);

    let rs_entry = files.iter().find(|f| f["path"] == "valid.rs").unwrap();
    assert_eq!(rs_entry["status"], "checked");
    assert_eq!(rs_entry["language"], "rust");

    let js_entry = files.iter().find(|f| f["path"] == "valid.js").unwrap();
    assert_eq!(js_entry["status"], "checked");
    assert_eq!(js_entry["language"], "javascript");

    let ts_entry = files.iter().find(|f| f["path"] == "valid.ts").unwrap();
    assert_eq!(ts_entry["status"], "checked");
    assert_eq!(ts_entry["language"], "typescript");

    let json_entry = files.iter().find(|f| f["path"] == "valid.json").unwrap();
    assert_eq!(json_entry["status"], "checked");
    assert_eq!(json_entry["language"], "json");

    let md_entry = files.iter().find(|f| f["path"] == "readme.md").unwrap();
    assert_eq!(md_entry["status"], "skipped");
    assert_eq!(md_entry["reason"], "unsupported_language");
}

#[test]
fn test_diagnostics_all_checked_is_complete_and_ok() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    fs::write(root.join("a.rs"), "pub fn a() -> u32 { 1 }\n").unwrap();
    fs::write(root.join("b.ts"), "export const b: string = 'hello';\n").unwrap();

    let app = adapters::application(root).unwrap();
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        session: None,
        include_rules: None,
    }));
    let session = boot["data"]["sessionId"].as_str().unwrap().to_string();

    let res = app.execute(Request::Diagnostics(DiagnosticsArgs {
        session: Some(session),
        files: Some(vec!["a.rs".into(), "b.ts".into()]),
        max_files: Some(10),
        build_scope: None,
    }));

    assert_eq!(res["state"], "ok");
    assert_eq!(res["data"]["pass"], true);
    assert_eq!(res["data"]["complete"], true);
    assert_eq!(res["data"]["status"], "no_errors_in_checked_scope");
    assert_eq!(res["data"]["errors"], 0);
}

#[test]
fn test_diagnostics_catches_syntax_errors() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    fs::write(root.join("bad.rs"), "fn broken( {\n").unwrap();
    fs::write(root.join("bad.ts"), "const x: = ;\n").unwrap();
    fs::write(root.join("bad.json"), "{ invalid: json }\n").unwrap();

    let app = adapters::application(root).unwrap();
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        session: None,
        include_rules: None,
    }));
    let session = boot["data"]["sessionId"].as_str().unwrap().to_string();

    let res = app.execute(Request::Diagnostics(DiagnosticsArgs {
        session: Some(session),
        files: Some(vec!["bad.rs".into(), "bad.ts".into(), "bad.json".into()]),
        max_files: Some(10),
        build_scope: None,
    }));

    assert_eq!(res["state"], "ok");
    assert_eq!(res["data"]["pass"], false);
    assert_eq!(res["data"]["status"], "issues_found");
    assert_eq!(res["data"]["errors"], 3);

    let files = res["data"]["files"].as_array().unwrap();
    for f in files {
        assert_eq!(f["status"], "checked");
        assert_eq!(f["errors"], 1);
        assert!(!f["diagnostics"].as_array().unwrap().is_empty());
    }
}

#[test]
fn test_diagnostics_validates_argument_bounds() {
    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        session: None,
        include_rules: None,
    }));
    let session = boot["data"]["sessionId"].as_str().unwrap().to_string();

    // 0 files
    let res = app.execute(Request::Diagnostics(DiagnosticsArgs {
        session: Some(session.clone()),
        files: Some(vec![]),
        max_files: None,
        build_scope: None,
    }));
    assert_eq!(res["state"], "error");
    assert_eq!(res["diagnostics"][0]["code"], "ARGUMENT");

    // invalid max_files
    let res = app.execute(Request::Diagnostics(DiagnosticsArgs {
        session: Some(session.clone()),
        files: Some(vec!["a.rs".into()]),
        max_files: Some(0),
        build_scope: None,
    }));
    assert_eq!(res["state"], "error");
    assert_eq!(res["diagnostics"][0]["code"], "ARGUMENT");

    // invalid build_scope
    let res = app.execute(Request::Diagnostics(DiagnosticsArgs {
        session: Some(session),
        files: Some(vec!["a.rs".into()]),
        max_files: Some(10),
        build_scope: Some("invalid.txt".into()),
    }));
    assert_eq!(res["state"], "error");
    assert_eq!(res["diagnostics"][0]["code"], "ARGUMENT");
}
