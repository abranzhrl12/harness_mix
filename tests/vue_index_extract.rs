use agent_harness::adapters::index_extract::extract;
use harness_domain::execution::OperationContext;
use serde_json::Value;

fn extract_vue(source: &str) -> Value {
    extract("src/Parent.vue", source, &OperationContext::default()).unwrap()
}

fn node<'a>(graph: &'a Value, name: &str, kind: &str) -> &'a Value {
    graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == name && item["kind"] == kind)
        .unwrap_or_else(|| panic!("missing {kind} node {name}: {graph}"))
}

fn has_contains(graph: &Value, source: &str, target: &str) -> bool {
    graph["edges"].as_array().unwrap().iter().any(|edge| {
        edge["source"] == source && edge["target"] == target && edge["kind"] == "contains"
    })
}

#[test]
fn vue_extracts_ts_script_refs_and_sfc_structure_with_utf16_positions() {
    let source = "<!-- 😀 -->\r\n<script lang=\"ts\">\r\nimport { greet } from './greet'\r\nexport function run() { const label = '😀'; greet(label); }\r\n</script>\r\n<template><div><CardView :title=\"label\" /></div></template>\r\n<style scoped>.card { color: red; &__title { font-weight: bold; } }</style>";
    let graph = extract_vue(source);

    let file = node(&graph, "src/Parent.vue", "file");
    let component_definition = node(&graph, "Parent", "component");
    let script = node(&graph, "script#0", "module");
    let template = node(&graph, "template#1", "module");
    let style = node(&graph, "style#2", "module");
    let component_tag = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "CardView" && item["componentTagCandidate"] == true)
        .unwrap();
    assert!(
        component_definition["id"]
            .as_str()
            .unwrap()
            .starts_with("native:framework:")
    );
    assert!(component_tag["kind"] == "module");
    assert_eq!(
        graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["kind"] == "component")
            .count(),
        1
    );
    assert!(node(&graph, "run", "function")["startLine"] == 4);
    assert!(node(&graph, ".card", "module")["language"] == "vue");
    assert!(has_contains(
        &graph,
        file["id"].as_str().unwrap(),
        component_definition["id"].as_str().unwrap()
    ));
    assert!(has_contains(
        &graph,
        component_definition["id"].as_str().unwrap(),
        script["id"].as_str().unwrap()
    ));
    assert!(has_contains(
        &graph,
        component_definition["id"].as_str().unwrap(),
        template["id"].as_str().unwrap()
    ));
    assert!(has_contains(
        &graph,
        component_definition["id"].as_str().unwrap(),
        style["id"].as_str().unwrap()
    ));
    assert!(has_contains(
        &graph,
        template["id"].as_str().unwrap(),
        component_tag["id"].as_str().unwrap()
    ));

    let refs = graph["unresolvedReferences"].as_array().unwrap();
    let component_ref = refs
        .iter()
        .find(|r| r["referenceName"] == "CardView" && r["referenceKind"] == "vue_template_component")
        .unwrap();
    assert_eq!(component_ref["filePath"], "src/Parent.vue");
    assert_eq!(component_ref["line"], 6);
    let imported = refs
        .iter()
        .find(|r| r["referenceName"] == "greet" && r["referenceKind"] == "imports")
        .unwrap();
    assert_eq!(imported["line"], 3);
    let call = refs
        .iter()
        .find(|r| r["referenceName"] == "greet" && r["line"] == 4)
        .unwrap();
    let line = source.split('\n').nth(3).unwrap().trim_end_matches('\r');
    let expected_column = line[..line.rfind("greet").unwrap()].encode_utf16().count();
    assert_eq!(call["column"], expected_column);

    assert_eq!(graph["coverage"]["scripts"]["references"], "script-only");
    assert_eq!(graph["coverage"]["template"]["expressionsResolved"], false);
    assert_eq!(
        graph["coverage"]["template"]["componentImportsResolved"],
        false
    );
    assert_eq!(
        graph["coverage"]["styles"]["selectorSemanticsResolved"],
        false
    );
}

#[test]
fn vue_template_component_candidates_respect_the_parser_item_limit() {
    let source = format!("<template>{}</template>", "<CardView/>".repeat(20_000));
    let error = extract("src/Parent.vue", &source, &OperationContext::default()).unwrap_err();
    assert_eq!(error.0, "PARSER_LIMIT");
}

#[test]
fn vue_extracts_js_and_setup_ts_blocks_and_is_deterministic() {
    let source = "<script>function shared() { ping(); }</script><script setup lang=\"ts\">function shared() { pong(); }</script>";
    let first = extract_vue(source);
    let second = extract_vue(source);
    assert_eq!(first, second);
    let functions = first["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["name"] == "shared" && item["kind"] == "function")
        .collect::<Vec<_>>();
    assert_eq!(functions.len(), 2);
    assert_ne!(functions[0]["id"], functions[1]["id"]);
    let script_nodes = first["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "module" && item["blockKind"] == "script")
        .count();
    assert_eq!(script_nodes, 2);
    let refs = first["unresolvedReferences"].as_array().unwrap();
    assert!(refs.iter().any(|r| r["referenceName"] == "ping"));
    assert!(refs.iter().any(|r| r["referenceName"] == "pong"));
    assert!(
        first["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["filePath"] == "src/Parent.vue" && n["language"] == "vue")
    );
    let ids = first["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), first["nodes"].as_array().unwrap().len());
}

#[test]
fn vue_external_and_custom_blocks_are_reported_without_loading_files() {
    let source = "<script src=\"./external.ts\"/><script setup lang=\"coffee\">not parsed</script><template src=\"./external.html\"/><i18n lang=\"json\">{\"hello\":\"world\"}</i18n>";
    let graph = extract_vue(source);
    let notices = graph["coverage"]["notices"].as_array().unwrap();
    assert!(
        notices
            .iter()
            .any(|notice| notice.as_str().unwrap().contains("External block source"))
    );
    assert!(
        notices
            .iter()
            .any(|notice| notice.as_str().unwrap().contains("Custom <i18n>"))
    );
    assert!(graph["unresolvedReferences"].as_array().unwrap().is_empty());
    assert!(graph["coverage"]["scripts"]["nativeKernel"] == true);
    assert_eq!(graph["coverage"]["scripts"]["externalBlocksNotRead"], 1);
    assert_eq!(graph["coverage"]["externalBlocksNotRead"], 2);
    assert_eq!(
        graph["coverage"]["scripts"]["unsupportedLanguagesSkipped"],
        1
    );
    assert_eq!(graph["coverage"]["customBlocks"]["contentParsed"], false);
    assert_eq!(graph["coverage"]["vueCompilerOrTypeCheckerRun"], false);
}

#[test]
fn vue_invalid_script_structure_and_cancellation_fail_closed() {
    assert!(
        extract(
            "src/Broken.vue",
            "<template><div></template>",
            &OperationContext::default()
        )
        .is_err()
    );
    assert!(
        extract(
            "src/Broken.vue",
            "<script lang=\"ts\">const = ;</script>",
            &OperationContext::default()
        )
        .is_err()
    );
    let context = OperationContext::default();
    context.cancel();
    assert_eq!(
        extract("src/Parent.vue", "<template/>", &context)
            .unwrap_err()
            .0,
        "CANCELLED"
    );
}

#[test]
fn vue_extracts_provide_and_inject_literal_references() {
    let source = r#"<script setup lang="ts">
import { provide, inject } from 'vue';
provide('themeConfig', { dark: true });
const config = inject('themeConfig');
</script>
<template><div /></template>"#;
    let graph = extract_vue(source);
    let refs = graph["unresolvedReferences"].as_array().unwrap();

    let provide_ref = refs.iter().find(|r| r["referenceName"] == "themeConfig" && r["referenceKind"] == "vue_provide");
    assert!(provide_ref.is_some(), "Debe extraer provide con clave 'themeConfig': {:?}", refs);

    let inject_ref = refs.iter().find(|r| r["referenceName"] == "themeConfig" && r["referenceKind"] == "vue_inject");
    assert!(inject_ref.is_some(), "Debe extraer inject con clave 'themeConfig': {:?}", refs);
}


#[test]
fn vue_router_extracts_routes_and_component_references() {
    let source = r#"
import { createRouter, createWebHistory } from 'vue-router';
import HomeView from './views/HomeView.vue';

const routes = [
  {
    path: '/',
    name: 'home',
    component: HomeView
  },
  {
    path: '/about',
    name: 'about',
    component: () => import('./views/AboutView.vue')
  }
];

export const router = createRouter({
  history: createWebHistory(),
  routes
});
"#;
    let graph = extract("src/router/index.ts", source, &OperationContext::default()).unwrap();
    let nodes = graph["nodes"].as_array().unwrap();
    let refs = graph["unresolvedReferences"].as_array().unwrap();

    let route_home = nodes.iter().find(|n| n["kind"] == "route" && n["name"] == "/");
    assert!(route_home.is_some(), "Debe emitir el nodo route '/': {:?}", nodes);

    let route_about = nodes.iter().find(|n| n["kind"] == "route" && n["name"] == "/about");
    assert!(route_about.is_some(), "Debe emitir el nodo route '/about': {:?}", nodes);

    let home_ref = refs.iter().find(|r| r["referenceName"] == "HomeView" && r["referenceKind"] == "vue_route_component");
    assert!(home_ref.is_some(), "Debe emitir la referencia estática a HomeView: {:?}", refs);

    let about_ref = refs.iter().find(|r| r["referenceName"] == "./views/AboutView.vue" && r["referenceKind"] == "vue_route_dynamic_component");
    assert!(about_ref.is_some(), "Debe emitir la referencia dinámica a AboutView: {:?}", refs);
}
