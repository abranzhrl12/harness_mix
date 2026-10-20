use agent_harness::adapters::index_extract::{extract, language};
use harness_domain::execution::OperationContext;
use serde_json::{Value, json};

fn cases() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "src/lib.rs",
            "pub struct Widget;\nimpl Widget { pub fn run(&self) { helper(); } }\npub fn helper() {}\n",
        ),
        (
            "src/Example.cs",
            "namespace Demo;\npublic class Example { public void Run() { Helper(); } public void Helper() {} }\n",
        ),
        (
            "src/conditional.cs",
            "namespace Demo;\r\n#if FLAG // 😀\r\npublic class First {}\r\n#else\r\npublic class Second {}\r\n#endif\r\n",
        ),
        (
            "src/app.js",
            "export function helper() {}\nexport function run() { helper(); }\n",
        ),
        (
            "src/app.jsx",
            "export function App() { return <div>Hello</div>; }\n",
        ),
        (
            "src/app.ts",
            "export interface Item { name: string }\nexport function run(item: Item): string { return item.name; }\n",
        ),
        (
            "src/app.tsx",
            "export function App() { return <div>Hello</div>; }\n",
        ),
    ]
}

#[test]
fn six_languages_extract_provider_nodes_edges_and_references_without_node() {
    for (path, source) in cases() {
        let result = extract(path, source, &OperationContext::default()).unwrap();
        let nodes = result["nodes"].as_array().unwrap();
        assert!(nodes.len() >= 2, "{path}: {result}");
        for node in nodes {
            assert_eq!(node["filePath"], path);
            assert_eq!(node["language"], language(path).unwrap());
            let id = node["id"].as_str().unwrap();
            assert!(!id.is_empty());
            assert!(node["startLine"].as_u64().unwrap() >= 1);
        }
        assert_eq!(result["errors"], json!([]));
    }
    let result = extract(
        "calls.rs",
        "fn first() { second(); }\nfn second() {}",
        &OperationContext::default(),
    )
    .unwrap();
    assert!(
        result["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["kind"] == "contains")
    );
    assert!(
        result["unresolvedReferences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reference| reference["referenceName"] == "second")
    );
}

#[test]
fn razor_declarations_enter_the_graph_without_markup_call_guesses() {
    let source = "@model Demo.Model\r\n@section Scripts { <script>fake();</script> }\r\n@functions {\r\n public string Hello() { return \"á😀\"; }\r\n}\r\n";
    let result = extract("Views/Index.cshtml", source, &OperationContext::default()).unwrap();
    let nodes = result["nodes"].as_array().unwrap();
    assert_eq!(language("Views/Index.cshtml"), Some("razor"));
    for (name, kind, line) in [
        ("Demo.Model", "type", 1),
        ("Scripts", "module", 2),
        ("Hello", "method", 4),
    ] {
        let node = nodes.iter().find(|n| n["name"] == name).unwrap();
        assert_eq!(node["kind"], kind);
        assert_eq!(node["startLine"], line);
        assert_eq!(node["language"], "razor");
        assert!(
            result["edges"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["target"] == node["id"] && e["kind"] == "contains")
        );
    }
    assert!(!nodes.iter().any(|n| n["name"] == "fake"));
    assert!(
        result["unresolvedReferences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["referenceName"] == "Demo.Model" && r["referenceKind"] == "references")
    );
    assert!(
        extract(
            "bad.cshtml",
            "@section Scripts {",
            &OperationContext::default()
        )
        .is_err()
    );
    let repeated = extract("Views/Index.cshtml", source, &OperationContext::default()).unwrap();
    assert_eq!(result, repeated);
    let other = extract("Views/Other.cshtml", source, &OperationContext::default()).unwrap();
    assert_ne!(result["nodes"][1]["id"], other["nodes"][1]["id"]);
}

#[test]
fn razor_native_member_projection_preserves_calls_fields_and_positions() {
    let source = "@* @functions { void Fake() {} } *@\r\n<div>😀</div>\r\n@functions {\r\n private int counter = 0;\r\n public void Run() { Helper(); }\r\n}\r\n@code {\r\n public void Helper() { counter++; }\r\n}\r\n";
    let result = extract("Views/Index.CSHTML", source, &OperationContext::default()).unwrap();
    let nodes = result["nodes"].as_array().unwrap();
    assert!(
        !nodes
            .iter()
            .any(|n| n["name"] == "Fake" || n["name"] == "__HarnessRazor")
    );
    for (name, line) in [("counter", 4), ("Run", 5), ("Helper", 8)] {
        let node = nodes.iter().find(|n| n["name"] == name).unwrap();
        assert_eq!(node["startLine"], line, "{result}");
        assert_eq!(node["filePath"], "Views/Index.CSHTML");
    }
    let run = nodes.iter().find(|n| n["name"] == "Run").unwrap();
    assert!(
        result["unresolvedReferences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["fromNodeId"] == run["id"]
                && r["referenceName"] == "Helper"
                && r["line"] == 5)
    );
    assert!(
        extract(
            "bad.cshtml",
            "@functions { public void Run( {} }",
            &OperationContext::default()
        )
        .is_err()
    );
    let context = OperationContext::default();
    context.cancel();
    assert_eq!(
        extract("view.cshtml", source, &context).unwrap_err().0,
        "CANCELLED"
    );
}

#[test]
fn razor_does_not_index_directive_text_in_markup() {
    let source = r#"<!-- @functions { void Comment() {} } -->
<script>const text = '@functions { void Script() {} }';</script>
<style>p::after { content: '@functions { void Style() {} }'; }</style>
<div title='@functions { void Attribute() {} }'></div>
@functions { public void Real() {} }"#;
    let result = extract("Index.cshtml", source, &OperationContext::default()).unwrap();
    let names = result["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["name"].as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Real"), "{result}");
    for fake in ["Comment", "Script", "Style", "Attribute"] {
        assert!(!names.contains(&fake), "{result}");
    }
}

#[test]
fn parser_errors_and_unsupported_languages_do_not_publish_partial_graphs() {
    assert_eq!(
        extract("broken.rs", "fn broken( {", &OperationContext::default())
            .unwrap_err()
            .0,
        "EXTRACTION_DEFERRED"
    );
    assert_eq!(
        extract("other.py", "def f(): pass", &OperationContext::default())
            .unwrap_err()
            .0,
        "SYNC_LANGUAGE"
    );
    let source = "x".repeat(1024 * 1024 + 1);
    assert!(extract("huge.rs", &source, &OperationContext::default()).is_err());
}

#[test]
#[ignore = "requires HARNESS_CODEGRAPH_PACKAGE and HARNESS_CODEGRAPH_NODE for provider parity only"]
fn extraction_matches_the_installed_codegraph_provider() {
    use std::process::Command;
    let package = std::env::var("HARNESS_CODEGRAPH_PACKAGE").expect("provider package root");
    let node = std::env::var("HARNESS_CODEGRAPH_NODE").expect("reference Node executable");
    let script = r#"
const path = require('path');
const root = process.argv[1];
const input = JSON.parse(process.argv[2]);
const api = require(path.join(root, 'dist/extraction/index.js'));
(async () => {
  await api.initGrammars();
  await api.loadGrammarsForLanguages([input.language]);
  const result = api.extractFromSource(input.path, input.source, input.language);
  for (const n of result.nodes) delete n.updatedAt;
  delete result.durationMs;
  process.stdout.write(JSON.stringify(result));
})().catch(e => { console.error(e); process.exitCode = 1; });
"#;
    for (path, source) in cases() {
        let input = json!({"path":path,"source":source,"language":language(path)});
        let output = Command::new(&node)
            .env("CODEGRAPH_KERNEL", "0")
            .args(["-e", script, &package, &input.to_string()])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{path}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected: Value = serde_json::from_slice(&output.stdout).unwrap();
        let actual = extract(path, source, &OperationContext::default()).unwrap();
        assert_eq!(actual, expected, "native vs CodeGraph wasm: {path}");
    }
}
