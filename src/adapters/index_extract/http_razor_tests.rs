use super::enrich;
use harness_domain::execution::OperationContext;
use serde_json::{Value, json};

fn graph(path: &str, lines: usize) -> Value {
    json!({
        "nodes":[{
            "id":format!("razor:file:{path}"),"kind":"file","name":path,
            "filePath":path,"language":"razor","startLine":1,"startColumn":0,
            "endLine":lines,"endColumn":0
        }],
        "edges":[],
        "unresolvedReferences":[]
    })
}

fn routes(graph: &Value) -> Vec<Value> {
    graph["unresolvedReferences"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|reference| reference["referenceKind"] == "http_route")
        .cloned()
        .collect()
}

fn parsed(reference: &Value) -> Value {
    serde_json::from_str(reference["referenceName"].as_str().unwrap()).unwrap()
}

#[test]
fn extracts_static_razor_urls_inside_inline_http_requests_with_method_and_caller() {
    let source = r#"<script>
async function load() {
  fetch('@Url.Action("Details", "Orders")');
  axios.post('@Url.Action("Save", "Orders")', body);
  fetch(dynamicUrl);
}
</script>"#;
    let mut output = graph("Views/Orders/Index.cshtml", 7);
    enrich(
        "Views/Orders/Index.cshtml",
        source,
        &mut output,
        &OperationContext::default(),
    )
    .unwrap();

    let references = routes(&output);
    assert_eq!(references.len(), 2);
    assert_eq!(
        parsed(&references[0]),
        json!({"url":"/Orders/Details","method":"GET"})
    );
    assert_eq!(
        parsed(&references[1]),
        json!({"url":"/Orders/Save","method":"POST"})
    );
    assert_eq!(references[0]["line"], 3);
    assert_eq!(references[1]["line"], 4);
    assert!(
        references
            .iter()
            .all(|reference| reference["filePath"] == "Views/Orders/Index.cshtml")
    );
    let caller = references[0]["fromNodeId"].as_str().unwrap();
    assert_ne!(caller, "razor:file:Views/Orders/Index.cshtml");
    assert!(
        output["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["id"] == caller
                && node["name"] == "load"
                && node["language"] == "razor")
    );
}

#[test]
fn html_scans_only_inline_javascript_and_ignores_non_javascript_or_external_scripts() {
    let source = r#"
<div data-note="fetch('/attribute')">fetch('/markup')</div>
<script type = "application/json">fetch('/json')</script>
<script src = "/app.js">fetch('/external')</script>
<script>
function go() { fetch('/inline'); }
</script>
"#;
    let mut output = graph("Pages/index.html", 8);
    enrich(
        "Pages/index.html",
        source,
        &mut output,
        &OperationContext::default(),
    )
    .unwrap();
    let references = routes(&output);
    assert_eq!(references.len(), 1);
    assert_eq!(
        parsed(&references[0]),
        json!({"url":"/inline","method":"GET"})
    );
    assert_eq!(references[0]["line"], 6);
    assert_eq!(references[0]["filePath"], "Pages/index.html");
}

#[test]
fn multiline_helpers_unicode_and_inline_razor_comments_keep_source_positions() {
    let source = r#"<script>
const emoji = "🧪";
function load() {
  fetch('@Url.Action(
    "Details",
    "Orders"
  )');
  fetch('@Url.Action(action, "Orders")');
  @* fetch('/ignored') *@
  fetch('/after');
}
</script>"#;
    let mut output = graph("Views/Orders/Index.cshtml", 12);
    enrich(
        "Views/Orders/Index.cshtml",
        source,
        &mut output,
        &OperationContext::default(),
    )
    .unwrap();
    let references = routes(&output);
    assert_eq!(references.len(), 2);
    assert_eq!(parsed(&references[0])["url"], "/Orders/Details");
    assert_eq!(references[0]["line"], 4);
    assert_eq!(parsed(&references[1])["url"], "/after");
    assert_eq!(references[1]["line"], 10);
    assert_eq!(references[0]["fromNodeId"], references[1]["fromNodeId"]);
    assert!(output["nodes"].as_array().unwrap().iter().any(|node|
        node["id"] == references[0]["fromNodeId"] && node["name"] == "load"));
}

#[test]
fn does_not_extract_script_text_inside_html_or_razor_comments() {
    let source = r#"<!-- <script>fetch('/html-comment')</script> -->
@* <script>fetch('/razor-comment')</script> *@
<script>fetch('/real');</script>"#;
    let mut output = graph("Views/Home/Index.cshtml", 3);
    enrich(
        "Views/Home/Index.cshtml",
        source,
        &mut output,
        &OperationContext::default(),
    )
    .unwrap();
    let references = routes(&output);
    assert_eq!(references.len(), 1);
    assert_eq!(parsed(&references[0])["url"], "/real");
}
