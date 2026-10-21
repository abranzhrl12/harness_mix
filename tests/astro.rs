use agent_harness::adapters;
use harness_domain::{contracts::Request, hash};
use serde_json::json;

#[test]
fn astro_outline_and_symbol_reading_works() {
    let root = tempfile::tempdir().unwrap();
    let astro_content = r#"---
import Card from '../components/Card.astro';
import type { UserProfile } from '../types';

export interface Props {
  title: string;
  count?: number;
}

const { title, count = 0 } = Astro.props;

function formatTitle(t: string): string {
  return t.toUpperCase();
}
---

<div class="card-container">
  <h1>{formatTitle(title)}</h1>
  <p>Count: {count}</p>
  <Card title={title} />
</div>

<style>
  .card-container {
    padding: 1rem;
  }
</style>
"#;
    std::fs::write(root.path().join("TestComponent.astro"), astro_content).unwrap();

    let app = adapters::application(root.path()).unwrap();
    let outline_req = Request::Outline(
        serde_json::from_value(json!({
            "path": "TestComponent.astro"
        }))
        .unwrap(),
    );
    let outline_res = app.execute(outline_req);
    assert_eq!(outline_res["state"], "ok", "{outline_res}");
    let data = &outline_res["data"];
    assert_eq!(data["language"], "astro");
    assert_eq!(data["syntaxOnly"], true);

    let items = data["items"].as_array().unwrap();
    assert!(
        items
            .iter()
            .any(|s| s["name"] == "Props" && s["kind"] == "interface_declaration")
    );
    assert!(
        items
            .iter()
            .any(|s| s["name"] == "formatTitle" && s["kind"] == "function_declaration")
    );
    assert!(
        items
            .iter()
            .any(|s| s["kind"] == "astro_import" && s["name"] == "import ../components/Card.astro")
    );

    // Now test reading a specific symbol: formatTitle
    let read_req = Request::ReadSymbol(
        serde_json::from_value(json!({
            "path": "TestComponent.astro",
            "symbol": "formatTitle",
            "view": "full"
        }))
        .unwrap(),
    );
    let read_res = app.execute(read_req);
    assert_eq!(read_res["state"], "ok", "{read_res}");
    assert!(
        read_res["data"]["text"]
            .as_str()
            .unwrap()
            .contains("function formatTitle")
    );
}

#[test]
fn astro_write_and_replace_with_syntax_validation() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();

    let initial = r#"---
const greeting = "Hello";
---
<div>{greeting}</div>
"#;
    let write_req = Request::Write(
        serde_json::from_value(json!({
            "path": "src/Page.astro",
            "content": initial,
            "createParents": true
        }))
        .unwrap(),
    );
    let write_res = app.execute(write_req);
    assert_eq!(write_res["state"], "ok", "{write_res}");

    // Test successful replace
    let initial_hash = hash(initial.as_bytes());
    let replaced_content = r#"---
const greeting = "Hello World";
---
<div>{greeting}</div>
"#;
    let replace_req = Request::Replace(
        serde_json::from_value(json!({
            "path": "src/Page.astro",
            "hash": initial_hash,
            "oldText": initial,
            "newText": replaced_content
        }))
        .unwrap(),
    );
    let replace_res = app.execute(replace_req);
    assert_eq!(replace_res["state"], "ok", "{replace_res}");

    // Test rejecting invalid TypeScript syntax in frontmatter
    let current_hash = hash(replaced_content.as_bytes());
    let bad_ts = r#"---
const invalid: = ;
---
<div>Error</div>
"#;
    let bad_replace_req = Request::Replace(
        serde_json::from_value(json!({
            "path": "src/Page.astro",
            "hash": current_hash,
            "oldText": replaced_content,
            "newText": bad_ts
        }))
        .unwrap(),
    );
    let bad_replace_res = app.execute(bad_replace_req);
    assert_eq!(bad_replace_res["state"], "error");
    assert_eq!(bad_replace_res["diagnostics"][0]["code"], "SYNTAX_ERROR");

    // Test rejecting unterminated frontmatter
    let unterminated = r#"---
const test = 123;
<div>Never closed</div>
"#;
    let unterminated_req = Request::Replace(
        serde_json::from_value(json!({
            "path": "src/Page.astro",
            "hash": current_hash,
            "oldText": replaced_content,
            "newText": unterminated
        }))
        .unwrap(),
    );
    let unterminated_res = app.execute(unterminated_req);
    assert_eq!(unterminated_res["state"], "error");
    assert_eq!(unterminated_res["diagnostics"][0]["code"], "SYNTAX_ERROR");
}

#[test]
fn astro_diagnostics_reports_clean_or_errors() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();

    let valid_astro = "---\nconst ok = true;\n---\n<p>{ok}</p>";
    std::fs::write(root.path().join("Valid.astro"), valid_astro).unwrap();

    let diag_req = Request::Diagnostics(
        serde_json::from_value(json!({
            "files": ["Valid.astro"]
        }))
        .unwrap(),
    );
    let diag_res = app.execute(diag_req);
    assert_eq!(diag_res["state"], "ok", "{diag_res}");
    let files = diag_res["data"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["errors"], 0);
}
