#![cfg(windows)]
use agent_harness::adapters;
use harness_domain::{
    contracts::{ProposalArgs, ReplaceArgs, Request, WriteArgs},
    hash,
};
use serde_json::{Value, json};
use std::fs;

#[test]
fn html_creation_preview_and_targeted_edit_need_no_opt_in_or_angular_dependencies() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    // Angular bindings and control flow must not be rejected by a generic HTML/XML parser.
    let template = "@if (visible()) {\r\n  <button [disabled]=\"busy()\" (click)=\"save()\">{{ label }}</button>\r\n}\r\n";
    for path in ["app.component.html", "index.htm", "page.HTML", "page.HTM"] {
        let args: WriteArgs =
            serde_json::from_value(json!({"path":path,"content":template})).unwrap();
        let created = app.execute(Request::Write(args));
        assert_eq!(created["state"], "ok", "{created}");
        assert_eq!(created["data"]["verified"], true);
        assert_eq!(created["data"]["syntax"]["status"], "not_checked");
        assert_eq!(created["data"]["syntax"]["validation"], "none");
        assert_eq!(created["data"]["syntax"]["language"], "html");
        assert_eq!(
            fs::read(root.path().join(path)).unwrap(),
            template.as_bytes()
        );

        let args: ReplaceArgs = serde_json::from_value(
            json!({"path":path,"oldText":"{{ label }}","newText":"{{ title() }}","preview":true}),
        )
        .unwrap();
        let preview = app.execute(Request::Replace(args));
        assert_eq!(preview["state"], "ok", "{preview}");
        assert_eq!(preview["data"]["published"], false);
        assert_eq!(
            fs::read(root.path().join(path)).unwrap(),
            template.as_bytes()
        );

        let args: ReplaceArgs = serde_json::from_value(json!({"path":path,"hash":hash(template.as_bytes()),"oldText":"{{ label }}","newText":"{{ title() }}"})).unwrap();
        let edited = app.execute(Request::Replace(args));
        assert_eq!(edited["state"], "ok", "{edited}");
        assert_eq!(edited["data"]["verified"], true);
        assert_eq!(edited["data"]["syntax"]["status"], "not_checked");
        assert_eq!(
            fs::read_to_string(root.path().join(path)).unwrap(),
            template.replace("{{ label }}", "{{ title() }}")
        );
    }
    assert!(!root.path().join("node_modules").exists());
}

#[test]
fn html_whole_file_replace_and_proposals_keep_bom_and_hash_protection() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let original = "\u{feff}<p>antes</p>\r\n";
    fs::write(root.path().join("file.html"), original).unwrap();
    let stale: WriteArgs = serde_json::from_value(
        json!({"path":"file.html","content":"<p>nuevo</p>\r\n","expectedHash":hash(b"older")}),
    )
    .unwrap();
    assert_eq!(app.execute(Request::Write(stale))["state"], "error");
    assert_eq!(
        fs::read(root.path().join("file.html")).unwrap(),
        original.as_bytes()
    );
    let args: WriteArgs = serde_json::from_value(json!({"path":"file.html","content":"<p>nuevo</p>\r\n","expectedHash":hash(original.as_bytes())})).unwrap();
    let result = app.execute(Request::Write(args));
    assert_eq!(result["state"], "ok", "{result}");
    let updated = "\u{feff}<p>nuevo</p>\r\n";
    assert_eq!(
        fs::read(root.path().join("file.html")).unwrap(),
        updated.as_bytes()
    );
    let args: ProposalArgs = serde_json::from_value(json!({"action":"prepare","path":"file.html","content":"<p>{{ next() }}</p>\r\n","expectedHash":hash(updated.as_bytes())})).unwrap();
    let proposal = app.execute(Request::Proposal(args));
    assert_eq!(proposal["state"], "ok", "{proposal}");
    assert_eq!(proposal["data"]["stored"], true);
    assert_eq!(proposal["data"]["published"], false);
    assert_eq!(proposal["data"]["syntax"]["status"], "not_checked");
    assert_eq!(
        fs::read(root.path().join("file.html")).unwrap(),
        updated.as_bytes()
    );
}

#[test]
fn html_text_support_does_not_silently_validate_templates_or_unlock_unknown_formats() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    // An unfinished editor buffer is allowed, but must never be reported as valid syntax.
    let unfinished = "<button [disabled]=\"{{";
    let args: WriteArgs =
        serde_json::from_value(json!({"path":"unfinished.html","content":unfinished})).unwrap();
    let result = app.execute(Request::Write(args));
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["syntax"]["status"], "not_checked");
    for (path, content) in [("unknown.xyz", "text"), ("broken.json", "{")] {
        let args: WriteArgs =
            serde_json::from_value(json!({"path":path,"content":content})).unwrap();
        let result = app.execute(Request::Write(args));
        assert_eq!(result["state"], "error", "{result}");
        assert!(!root.path().join(path).exists());
    }
    let args: WriteArgs =
        serde_json::from_value(json!({"path":"nul.html","content":"<p>\0</p>"})).unwrap();
    let result: Value = app.execute(Request::Write(args));
    assert_eq!(result["state"], "error");
    assert!(!root.path().join("nul.html").exists());
}
