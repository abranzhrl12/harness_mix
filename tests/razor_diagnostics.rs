use agent_harness::adapters;
use harness_domain::contracts::{DiagnosticsArgs, Request};
use std::fs;
use tempfile::tempdir;

#[test]
fn razor_doctor_reports_media_and_accepts_escaped_css() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("bad.cshtml"),
        "<style>@media screen { body { color: red; } }</style>",
    )
    .unwrap();
    fs::write(dir.path().join("good.cshtml"), "@section Styles { <style>@@media screen { body { color: red; } }</style> }<main><input disabled><br></main>").unwrap();
    let app = adapters::application(dir.path()).unwrap();
    let result = app.execute(Request::Diagnostics(DiagnosticsArgs {
        session: None,
        files: Some(vec!["bad.cshtml".into(), "good.cshtml".into()]),
        max_files: Some(10),
        build_scope: None,
    }));
    assert_eq!(result["state"], "ok", "{result}");
    let files = result["data"]["files"].as_array().unwrap();
    let bad = files.iter().find(|f| f["path"] == "bad.cshtml").unwrap();
    let good = files.iter().find(|f| f["path"] == "good.cshtml").unwrap();
    assert_eq!(bad["errors"], 1, "{result}");
    assert!(bad.to_string().contains("media"), "{result}");
    assert_eq!(good["errors"], 0, "{result}");
    assert_eq!(good["language"], "razor");
}
