#![cfg(windows)]
use agent_harness::adapters;
use harness_domain::contracts::{ReplaceArgs, Request};
use serde_json::json;

#[test]
fn hashless_replace_reads_internally_and_line_selector_is_validated_before_publication() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("style.txt");
    let source = "padding: 5px;\r\npadding: 5px;\r\n";
    std::fs::write(&path, source).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let replace = |line, preview| {
        let args: ReplaceArgs = serde_json::from_value(json!({
            "path":"style.txt", "oldText":"padding: 5px;",
            "newText":"padding: 8px;", "line":line, "preview":preview
        }))
        .unwrap();
        app.execute(Request::Replace(args))
    };
    let ambiguous = replace(None::<usize>, false);
    assert_eq!(ambiguous["diagnostics"][0]["code"], "MATCH", "{ambiguous}");
    let invalid = replace(Some(3), false);
    assert_eq!(invalid["diagnostics"][0]["code"], "MATCH", "{invalid}");
    let preview = replace(Some(2), true);
    assert_eq!(preview["state"], "ok", "{preview}");
    assert_eq!(preview["data"]["published"], false);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    let result = replace(Some(2), false);
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["verified"], true);
    let expected = "padding: 5px;\r\npadding: 8px;\r\n";
    assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
    assert_eq!(
        result["data"]["hash"],
        harness_domain::hash(expected.as_bytes())
    );
    let stale_line = replace(Some(2), false);
    assert_eq!(
        stale_line["diagnostics"][0]["code"], "MATCH",
        "{stale_line}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
    let unique = replace(None, false);
    assert_eq!(unique["state"], "ok", "{unique}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "padding: 8px;\r\npadding: 8px;\r\n"
    );
}
