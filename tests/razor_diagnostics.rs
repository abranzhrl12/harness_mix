use agent_harness::adapters;
use harness_domain::contracts::{DiagnosticsArgs, ReplaceArgs, Request};
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

#[test]
fn razor_attribute_errors_block_write_and_replace_without_changing_disk() {
    use serde_json::json;
    let root = tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let bad = r#"<input pattern="^[^@]+@[^@]+$" />"#;
    let good = r#"<input pattern="^[^@@]+@@[^@@]+$" value="@Model.Email" />"#;
    let rejected = app.execute(Request::Write(
        serde_json::from_value(json!({
            "path":"new.cshtml", "content":bad, "bom":false
        }))
        .unwrap(),
    ));
    assert!(
        rejected.to_string().contains("RAZOR_ATTRIBUTE_TRANSITION"),
        "{rejected}"
    );
    assert!(!root.path().join("new.cshtml").exists());
    let unquoted = app.execute(Request::Write(
        serde_json::from_value(json!({
            "path":"unquoted.cshtml", "content":"<input pattern=^[^@]+$>", "bom":false
        }))
        .unwrap(),
    ));
    assert!(
        unquoted.to_string().contains("RAZOR_ATTRIBUTE_TRANSITION"),
        "{unquoted}"
    );
    assert!(!root.path().join("unquoted.cshtml").exists());
    let unquoted = app.execute(Request::Write(serde_json::from_value(json!({
        "path":"unquoted.cshtml", "content":"<input value=@Model.Email pattern=@@ />", "bom":false
    })).unwrap()));
    assert_eq!(unquoted["state"], "ok", "{unquoted}");
    let created = app.execute(Request::Write(
        serde_json::from_value(json!({
            "path":"new.cshtml", "content":good, "bom":false
        }))
        .unwrap(),
    ));
    assert_eq!(created["state"], "ok", "{created}");
    let rejected = app.execute(Request::Replace(
        serde_json::from_value(json!({
            "path":"new.cshtml", "oldText":good, "newText":bad
        }))
        .unwrap(),
    ));
    assert!(
        rejected.to_string().contains("RAZOR_ATTRIBUTE_TRANSITION"),
        "{rejected}"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("new.cshtml")).unwrap(),
        good
    );
    let accepted = app.execute(Request::Replace(
        serde_json::from_value(json!({
            "path":"new.cshtml", "oldText":"@Model.Email", "newText":"@(Model.Email)"
        }))
        .unwrap(),
    ));
    assert_eq!(accepted["state"], "ok", "{accepted}");
    assert!(
        fs::read_to_string(root.path().join("new.cshtml"))
            .unwrap()
            .contains("@(Model.Email)")
    );
}

#[test]
fn razor_section_ignores_javascript_braces_and_opt_in_bypasses_checks() {
    use serde_json::json;
    let root = tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let valid = "@section Scripts {\n<script>\nfunction foo() { return /{/; }\n</script>\n}\n";
    fs::write(root.path().join("view.cshtml"), valid).unwrap();
    let replaced = app.execute(Request::Replace(ReplaceArgs {
        bom: None,
        path: "view.cshtml".into(),
        hash: harness_domain::hash(valid.as_bytes()),
        line: None,
        chunks: None,
        old_text: "function foo()".into(),
        new_text: "function bar()".into(),
        allow_unvalidated_text: false,
        preview: false,
        session: None,
    }));
    assert_eq!(replaced["state"], "ok", "{replaced}");
    assert!(
        fs::read_to_string(root.path().join("view.cshtml"))
            .unwrap()
            .contains("function bar()")
    );

    let bad = r#"<input pattern="^[^@]+@[^@]+$" />"#;
    fs::write(root.path().join("bad.cshtml"), bad).unwrap();
    let denied = app.execute(Request::Replace(
        serde_json::from_value(json!({
            "path":"bad.cshtml", "oldText":"pattern", "newText":"pattern "
        }))
        .unwrap(),
    ));
    assert!(
        denied.to_string().contains("RAZOR_ATTRIBUTE_TRANSITION"),
        "{denied}"
    );
    let allowed = app.execute(Request::Replace(
        serde_json::from_value(json!({
            "path":"bad.cshtml", "oldText":"pattern", "newText":"pattern ",
            "allowUnvalidatedText":true
        }))
        .unwrap(),
    ));
    assert_eq!(allowed["state"], "ok", "{allowed}");
    assert_eq!(allowed["data"]["syntax"]["status"], "not_checked");
    assert_eq!(
        allowed["data"]["syntax"]["skippedSyntaxError"],
        "RAZOR_ATTRIBUTE_TRANSITION"
    );
}
