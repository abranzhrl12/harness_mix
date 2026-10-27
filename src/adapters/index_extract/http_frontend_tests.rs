use super::enrich;
use harness_domain::execution::OperationContext;
use serde_json::json;

fn graph() -> serde_json::Value {
    json!({"nodes":[
        {"id":"file:x.ts","kind":"file","filePath":"x.ts","startLine":1,"startColumn":0,"endLine":20,"endColumn":0},
        {"id":"fn:load","kind":"function","name":"load","startLine":1,"startColumn":0,"endLine":11,"endColumn":1}
    ],"unresolvedReferences":[]})
}

#[test]
fn extracts_literal_http_calls_and_keeps_dynamic_and_comment_text_out() {
    let source = r#"
function load() {
  fetch('/users');
  fetch('/submit', { method: 'post' });
  axios.get("/items");
  axios.post('/items', body);
  $.getJSON('/lookup');
  $.ajax({ url: '/save', type: 'put' });
  fetch(base + '/dynamic');
  // axios.get('/comment')
}
"#;
    let mut output = graph();
    enrich("x.ts", source, &mut output, &OperationContext::default()).unwrap();
    let refs = output["unresolvedReferences"].as_array().unwrap();
    assert_eq!(refs.len(), 6);
    for (reference, expected) in refs.iter().zip([
        json!({"url":"/users","method":"GET"}),
        json!({"url":"/submit","method":"POST"}),
        json!({"url":"/items","method":"GET"}),
        json!({"url":"/items","method":"POST"}),
        json!({"url":"/lookup","method":"GET"}),
        json!({"url":"/save","method":"PUT"}),
    ]) {
        let name: serde_json::Value =
            serde_json::from_str(reference["referenceName"].as_str().unwrap()).unwrap();
        assert_eq!(name, expected);
    }
    assert!(
        refs.iter()
            .all(|reference| reference["fromNodeId"] == "fn:load")
    );
    assert!(
        refs.iter()
            .all(|reference| reference["referenceKind"] == "http_route")
    );
}

#[test]
fn vue_scans_script_blocks_but_not_template_text() {
    let source = r#"<template><button @click="fetch('/template')">go</button></template>
<script setup lang="ts">
fetch('/script');
</script>"#;
    let mut output = graph();
    enrich(
        "Page.vue",
        source,
        &mut output,
        &OperationContext::default(),
    )
    .unwrap();
    let refs = output["unresolvedReferences"].as_array().unwrap();
    assert_eq!(refs.len(), 1);
    let name: serde_json::Value =
        serde_json::from_str(refs[0]["referenceName"].as_str().unwrap()).unwrap();
    assert_eq!(name["url"], "/script");
    assert_eq!(refs[0]["line"], 3);
}

#[test]
fn literal_must_not_contain_escapes_and_plain_templates_are_supported() {
    let source = r#"fetch('/escaped\\path'); fetch('/literal'); fetch(`/template`); fetch('/dynamic', {method: method});"#;
    let mut output = graph();
    enrich("x.js", source, &mut output, &OperationContext::default()).unwrap();
    let refs = output["unresolvedReferences"].as_array().unwrap();
    assert_eq!(refs.len(), 2);
    let first: serde_json::Value =
        serde_json::from_str(refs[0]["referenceName"].as_str().unwrap()).unwrap();
    let second: serde_json::Value =
        serde_json::from_str(refs[1]["referenceName"].as_str().unwrap()).unwrap();
    assert_eq!(first["url"], "/literal");
    assert_eq!(second["url"], "/template");
}
#[test]
fn chooses_the_smallest_same_line_caller() {
    let source = "function outer(){ function inner(){fetch('/inner')} fetch('/outer') }";
    let inner_start = source.find("function inner").unwrap();
    let inner_end = source.find(" fetch('/outer')").unwrap();
    let mut output = json!({"nodes":[
        {"id":"file:x.js","kind":"file","filePath":"x.js","startLine":1,"startColumn":0,"endLine":1,"endColumn":source.len()},
        {"id":"outer","kind":"function","startLine":1,"startColumn":0,"endLine":1,"endColumn":source.len()},
        {"id":"inner","kind":"function","startLine":1,"startColumn":inner_start,"endLine":1,"endColumn":inner_end}
    ],"unresolvedReferences":[]});
    enrich("x.js", source, &mut output, &OperationContext::default()).unwrap();
    let refs = output["unresolvedReferences"].as_array().unwrap();
    assert_eq!(refs.len(), 2);
    for reference in refs {
        let name: serde_json::Value =
            serde_json::from_str(reference["referenceName"].as_str().unwrap()).unwrap();
        let expected = if name["url"] == "/inner" {
            "inner"
        } else {
            "outer"
        };
        assert_eq!(reference["fromNodeId"], expected);
    }
}
