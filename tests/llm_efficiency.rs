//! Response-size/answer-preservation probes, not estimates of model token usage.
use agent_harness::adapters;
use harness_domain::contracts::Request;
use serde_json::{Value, json};
#[path = "support/codegraph.rs"]
mod graph_fixture;

fn explore(app: &adapters::NativeApplication, view: &str, session: &str) -> Value {
    app.execute(Request::Explore(
        serde_json::from_value(json!({
        "query":"target","filePath":"lib.rs","view":view,"requireMatchingFiles":true,"session":session
        }))
        .unwrap(),
    ))
}
fn project(app: &adapters::NativeApplication, result: &Value, expression: &str) -> Value {
    let response = app.execute(Request::DataQuery(
        serde_json::from_value(json!({
            "resultRef":result["detail"]["ref"],"query":expression,"maxChars":16000
        }))
        .unwrap(),
    ));
    assert_eq!(response["state"], "ok", "{response}");
    response["data"]["result"].clone()
}
fn units(value: &Value) -> usize {
    harness_domain::results::utf16_len(&value.to_string())
}

#[test]
fn compact_exploration_keeps_answers_and_graph_recoverable() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    let source = format!(
        "pub fn target() {{\n{}\ncallee();\n}}\npub fn caller() {{ target(); }}\npub fn callee() {{}}\n",
        "let _unused = \"fixture payload for a long implementation, not needed for navigation\";\n"
            .repeat(180)
    );
    std::fs::write(root.path().join("lib.rs"), &source).unwrap();
    // Keep fixture locations and hashes accurate without using a production DB.
    let caller_line = source[..source.find("pub fn caller").unwrap()]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1;
    let callee_line = source[..source.find("pub fn callee").unwrap()]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1;
    db.execute(
        "UPDATE nodes SET start_line=?1 WHERE id='c'",
        [caller_line as i64],
    )
    .unwrap();
    db.execute(
        "UPDATE nodes SET start_line=?1 WHERE id='d'",
        [callee_line as i64],
    )
    .unwrap();
    db.execute(
        "UPDATE files SET content_hash=?1 WHERE path='lib.rs'",
        [harness_domain::hash(source.as_bytes())],
    )
    .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let context = app.execute(Request::Bootstrap(
        serde_json::from_value(json!({})).unwrap(),
    ));
    let session = context["data"]["sessionId"].as_str().unwrap();
    let full = explore(&app, "full", session);
    let compact = explore(&app, "summary", session);
    assert_eq!(full["state"], "ok", "{full}");
    assert_eq!(compact["state"], "ok", "{compact}");
    assert_eq!(compact["data"]["definitionView"], "signature");
    assert_eq!(compact["data"]["editReady"], false);
    assert_eq!(
        compact["data"]["fileFreshness"]["status"],
        "matches_observed_files"
    );
    assert_eq!(compact["data"]["absenceProven"], false);
    let relations = compact["data"]["graph"]["relations"].as_array().unwrap();
    assert!(
        relations
            .iter()
            .any(|r| r["source"]["name"] == "caller" && r["target"]["name"] == "target")
    );
    assert!(
        relations
            .iter()
            .any(|r| r["source"]["name"] == "target" && r["target"]["name"] == "callee")
    );
    let expression = "graph.{nodes:nodes,edges:edges}";
    assert_eq!(
        project(&app, &full, expression),
        project(&app, &compact, expression)
    );
    assert!(
        units(&compact) < units(&full) / 2,
        "summary should reduce this navigation fixture substantially"
    );
    println!(
        "{}",
        json!({"scenario":"same-navigation-task","fullUtf16":units(&full),"summaryUtf16":units(&compact),"reductionPercent":100.0*(1.0-units(&compact) as f64/units(&full) as f64),"initialContextUtf16":units(&context),"twoCallFullUtf16":units(&context)+units(&full),"twoCallCompactUtf16":units(&context)+units(&compact),"twoCallReductionPercent":100.0*(1.0-(units(&context)+units(&compact)) as f64/(units(&context)+units(&full)) as f64),"sameNodesAndEdges":true,"callerAndCalleeAvailableWithoutExtraCall":true,"unit":"canonical envelope UTF-16, not model tokens"})
    );
    let body = app.execute(Request::ReadSymbol(
        serde_json::from_value(json!({"path":"lib.rs","symbol":"target","view":"full"})).unwrap(),
    ));
    let exact = project(
        &app,
        &body,
        "{hash:hash,hasBody:contains(text, 'fixture payload'),bodyLength:length(text)}",
    );
    assert_eq!(exact["hash"], harness_domain::hash(source.as_bytes()));
    // For editing use the exact read/detail, never the summary as oldText.
    assert_eq!(exact["hasBody"], true);
    assert!(exact["bodyLength"].as_u64().unwrap() > 1000);
}

#[test]
fn reused_bootstrap_is_compact_but_catalog_remains_recoverable() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let first = app.execute(Request::Bootstrap(
        serde_json::from_value(json!({})).unwrap(),
    ));
    let session = first["data"]["sessionId"].as_str().unwrap();
    assert_eq!(
        first["data"]["toolContract"]["features"]["symbolNavigation"],
        json!(["rust", "javascript", "typescript"])
    );
    let warm = app.execute(Request::Bootstrap(
        serde_json::from_value(json!({"session":session})).unwrap(),
    ));
    assert_eq!(warm["state"], "ok", "{warm}");
    assert_eq!(warm["data"]["catalogUnchanged"], true);
    assert_eq!(warm["data"]["rulesUnchanged"], true);
    assert!(warm["data"].get("supportedTools").is_none());
    assert_eq!(
        project(&app, &warm, "catalog.supportedTools"),
        first["data"]["supportedTools"]
    );
    let refreshed = app.execute(Request::Bootstrap(
        serde_json::from_value(json!({"session":session,"includeRules":true})).unwrap(),
    ));
    assert_eq!(refreshed["state"], "ok", "{refreshed}");
    assert_eq!(
        project(&app, &warm, "catalog.toolContract"),
        refreshed["data"]["toolContract"]
    );
    assert!(
        refreshed["data"]["routing"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["defaults"]["view"] == "summary")
    );
    assert!(units(&warm) < units(&refreshed) / 2);
    println!(
        "{}",
        json!({"scenario":"session-context-reuse","fullRefreshUtf16":units(&refreshed),"compactReuseUtf16":units(&warm),"reductionPercent":100.0*(1.0-units(&warm) as f64/units(&refreshed) as f64),"catalogRecoveredEqual":true,"unit":"canonical envelope UTF-16, not model tokens"})
    );
}

#[test]
fn outline_inventory_does_not_require_loading_all_signatures() {
    let root = tempfile::tempdir().unwrap();
    let source = (0..80)
        .map(|i| format!("pub fn declaration_{i}() {{}}\n"))
        .collect::<String>();
    std::fs::write(root.path().join("many.rs"), source).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let response = app.execute(Request::Outline(
        serde_json::from_value(json!({"path":"many.rs","query":"declaration_79","maxChars":2500}))
            .unwrap(),
    ));
    assert_eq!(response["state"], "ok", "{response}");
    assert_eq!(response["data"]["inventory"]["total"], 80);
    assert_eq!(response["data"]["items"][0]["name"], "declaration_79");
    assert!(response["data"]["inventory"]["omitted"].as_u64().unwrap() > 0);
    assert_eq!(project(&app, &response, "length(items)"), 80);
}
