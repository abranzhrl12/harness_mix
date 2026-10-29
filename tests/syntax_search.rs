use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{BatchArgs, DetailArgs, Kind, Request, SearchArgs};
use serde_json::{Value, json};

fn search(app: &NativeApplication, query: &str, limit: usize) -> Value {
    app.execute(Request::Search(SearchArgs {
            override_project_exclusions: None,
        only_matching: None,
        ast: None,
        regex: None,
        case_sensitive: None,
        context_lines: None,
        kind: Kind::Syntax,
        query: query.into(),
        min_lines: None,
        globs: None,
        scope: Some(".".into()),
        limit: Some(limit),
        session: None,
            cursor: None,
            offset: None,
        external_scope: None,
    }))
}

#[test]
fn selects_ast_declarations_not_comments_strings_calls_or_local_items() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("a.rs"),
        r#"
// fn pretend() {}
const TEXT: &str = "fn pretend() {}";
pub fn real() { fn local() {} let _ = "fn pretend() {}"; }
mod inner { pub async fn real() {} }
trait T { fn real(); fn defaulted() {} }
struct S;
impl S { fn real() {} }
macro_rules! ignored { () => { fn pretend() {} }; }
"#,
    )
    .unwrap();
    std::fs::write(root.path().join("b.txt"), "fn pretend() {}").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = search(&app, "function_item:real", 40);
    assert_eq!(result["state"], "ok", "{result}");
    let matches = result["data"]["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 3);
    assert!(matches.iter().all(|m| m["name"] == "real"
        && m["hash"].as_str().unwrap().len() == 64
        && m["symbolId"].is_string()));
    assert!(
        matches
            .iter()
            .any(|m| m["container"].as_str().unwrap().contains("inner"))
    );
    assert!(
        matches
            .iter()
            .any(|m| m["container"].as_str().unwrap().contains("impl S"))
    );
    assert_eq!(result["data"]["syntaxOnly"], true);
    assert_eq!(result["data"]["complete"], true);
    for name in ["pretend", "local"] {
        assert!(
            search(&app, &format!("function_item:{name}"), 40)["data"]["matches"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(
        search(&app, "function_signature_item:real", 40)["data"]["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&app, "struct_item:S", 40)["data"]["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        search(&app, "function_item:Real", 40)["data"]["matches"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn reports_invalid_source_and_budgets_as_partial_without_parser_content() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("valid.rs"), "pub fn keep() {}").unwrap();
    std::fs::write(root.path().join("invalid.rs"), "fn malformed-private(").unwrap();
    std::fs::write(root.path().join("binary.rs"), b"fn\0private-binary").unwrap();
    std::fs::write(
        root.path().join("oversize.rs"),
        " ".repeat(2 * 1024 * 1024 + 1),
    )
    .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = search(&app, "function_item", 40);
    assert_eq!(result["state"], "ok");
    assert_eq!(result["data"]["complete"], false);
    assert_eq!(result["data"]["parseErrors"], 1);
    assert_eq!(result["data"]["skipped"], 3);
    assert_eq!(result["data"]["matches"].as_array().unwrap().len(), 1);
    assert!(!result.to_string().contains("malformed-private"));
    assert!(!result.to_string().contains("private-binary"));
}

#[test]
fn preserves_detail_batch_ignores_and_validates_selectors_before_scanning() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(".gitignore"), "ignored.rs\n").unwrap();
    std::fs::write(root.path().join("ignored.rs"), "fn ignored() {}").unwrap();
    std::fs::create_dir(root.path().join("secrets")).unwrap();
    std::fs::write(root.path().join("secrets/hidden.rs"), "fn hidden() {}").unwrap();
    std::fs::write(root.path().join("a.rs"), "fn first() {}\nfn second() {}\n").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = search(&app, "function_item", 1);
    assert_eq!(result["data"]["matches"].as_array().unwrap().len(), 1);
    let detail = app.execute(Request::Detail(DetailArgs {
        reference: result["detail"]["ref"].as_str().unwrap().into(),
        offset: None,
        max_chars: Some(16000),
        session: None,
    }));
    assert!(detail.to_string().contains("second"));
    assert!(!detail.to_string().contains("ignored.rs"));
    assert!(!detail.to_string().contains("hidden.rs"));
    let batch = app.execute(Request::Batch(serde_json::from_value::<BatchArgs>(json!({"jobs":[{
        "tool":"harness_search","arguments":{"kind":"syntax","query":"function_item:first","scope":"."}
    }]})).unwrap()));
    assert_eq!(batch["state"], "ok", "{batch}");
    assert!(batch.to_string().contains("first"));
    for query in [
        "",
        "fn $NAME() {}",
        "unknown_item",
        "function_item:",
        "function_item:x:y",
    ] {
        assert_eq!(
            search(&app, query, 40)["diagnostics"][0]["code"],
            "ARGUMENT"
        );
    }
}
