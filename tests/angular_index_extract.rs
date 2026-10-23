use agent_harness::adapters::index_extract::{extract, language};
use harness_domain::execution::OperationContext;
use serde_json::json;

#[test]
fn angular_named_alias_creates_component_and_external_template_reference() {
    let source = r#"import { Component as C } from '@angular/core';
@C({ selector: 'app-card', templateUrl: './card/card.html' })
export class CardComponent {}
"#;
    let graph = extract("src/card.ts", source, &OperationContext::default()).unwrap();
    let component = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "component" && node["name"] == "CardComponent")
        .unwrap();
    assert!(
        component["id"]
            .as_str()
            .unwrap()
            .starts_with("native:framework:")
    );
    assert_eq!(component["selector"], "app-card");
    assert!(
        graph["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["source"] == component["id"] && edge["kind"] == "decorates")
    );
    assert!(
        graph["unresolvedReferences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reference| reference["fromNodeId"] == component["id"]
                && reference["referenceKind"] == "framework_template"
                && reference["referenceName"] == "src/card/card.html")
    );
}

#[test]
fn angular_namespace_and_inline_static_template_keep_structural_facts() {
    let source = "import * as ng from '@angular/core';\r\n@ng.Component({selector: 'x-card', template: `<section *ngIf=\"ready\">Hola {{ title }} @if (show) {<b [title]=\"title\">á</b>}</section>`})\r\nexport class Card {}\r\n";
    let graph = extract("src/card.ts", source, &OperationContext::default()).unwrap();
    let nodes = graph["nodes"].as_array().unwrap();
    assert!(
        nodes
            .iter()
            .any(|node| node["kind"] == "component" && node["name"] == "Card")
    );
    assert!(
        nodes
            .iter()
            .any(|node| node["name"] == "section" && node["kind"] == "variable")
    );
    assert!(
        nodes
            .iter()
            .any(|node| node["name"] == "interpolation" && node["kind"] == "property")
    );
    assert!(nodes.iter().any(|node| {
        node["name"] == "control-flow"
            || node["signature"]
                .as_str()
                .is_some_and(|s| s.starts_with("@if"))
    }));
    assert!(
        nodes
            .iter()
            .any(|node| node["name"] == "[title]" && node["kind"] == "property")
    );
    assert!(
        graph["unresolvedReferences"]
            .as_array()
            .unwrap()
            .iter()
            .all(|reference| reference["referenceKind"] != "calls"
                || reference["referenceName"] != "title")
    );
}

#[test]
fn imported_angular_symbol_is_required_and_dynamic_or_escaped_templates_are_not_guessed() {
    for source in [
        "function Component(x: unknown) {} @Component({selector:'x-a', template:'<b>fake</b>'}) class Local {}",
        "import { Component } from 'not-angular'; @Component({selector:'x-a'}) class Local {}",
    ] {
        let graph = extract("local.ts", source, &OperationContext::default()).unwrap();
        assert!(
            !graph["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["kind"] == "component")
        );
    }
    let source = r#"import { Component } from '@angular/core';
@Component({selector:'x-a', template: '<b>\n</b>'}) class Escaped {}
"#;
    let graph = extract("escaped.ts", source, &OperationContext::default()).unwrap();
    assert!(
        graph["coverage"]["notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().contains("escaped"))
    );
    assert!(
        !graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "b")
    );
}

#[test]
fn imported_angular_metadata_roles_add_markers_without_claiming_di_resolution() {
    let source = r#"import { Injectable as Service, Directive } from '@angular/core';
import * as ng from '@angular/core';
@Service() export class ApiService {}
@Directive({ selector: '[appHint]' }) export class HintDirective {}
@ng.Pipe({ name: 'label' }) export class LabelPipe {}
@ng.NgModule({}) export class FeatureModule {}
"#;
    let graph = extract("src/roles.ts", source, &OperationContext::default()).unwrap();
    let markers = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["kind"] == "module" && node["decorators"].is_array())
        .collect::<Vec<_>>();
    assert!(
        markers
            .iter()
            .any(|node| node["name"] == "ApiService" && node["decorators"][0] == "Injectable")
    );
    assert!(
        markers
            .iter()
            .any(|node| node["name"] == "HintDirective" && node["decorators"][0] == "Directive")
    );
    assert!(
        markers
            .iter()
            .any(|node| node["name"] == "LabelPipe" && node["decorators"][0] == "Pipe")
    );
    assert!(
        markers
            .iter()
            .any(|node| node["name"] == "FeatureModule" && node["decorators"][0] == "NgModule")
    );
    assert!(
        graph["unresolvedReferences"]
            .as_array()
            .unwrap()
            .iter()
            .all(|reference| reference["referenceKind"] != "di")
    );
}

#[test]
fn generic_html_has_structural_nodes_without_angular_claims() {
    assert_eq!(language("page.HTML"), Some("html"));
    let graph = extract(
        "views/page.HTML",
        "<!-- á -->\r\n<main><input [value]=\"x\" /><p>{{ text }}</p></main>",
        &OperationContext::default(),
    )
    .unwrap();
    let nodes = graph["nodes"].as_array().unwrap();
    let file_nodes = nodes
        .iter()
        .filter(|node| node["kind"] == "file")
        .collect::<Vec<_>>();
    assert_eq!(file_nodes.len(), 1);
    assert_eq!(file_nodes[0]["id"], "file:views/page.HTML");
    assert_eq!(file_nodes[0]["language"], "html");
    assert!(!nodes.iter().any(|node| node["kind"] == "component"));
    assert!(nodes.iter().any(|node| node["name"] == "main"));
    assert!(
        graph["coverage"]["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("no Angular context"))
    );
    assert_eq!(graph["errors"], json!([]));

    for source in ["", "plain text only", "<!-- comment only -->"] {
        let empty = extract("empty.html", source, &OperationContext::default()).unwrap();
        assert_eq!(
            empty["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|node| node["kind"] == "file")
                .count(),
            1
        );
        assert_eq!(empty["errors"], json!([]));
    }
}

#[test]
fn angular_route_extraction_requires_a_registered_static_routes_table() {
    let source = r#"import type { Routes } from '@angular/router';
import { RouterModule } from '@angular/router';
import { Home } from './home';
import { Admin, Users, AdminHome } from './admin';
const routes: Routes = [
  { path: 'home', component: Home },
  { path: 'admin', component: Admin, children: [
    { path: 'users', component: Users },
    { path: '', component: AdminHome },
  ] },
  { path: '**', redirectTo: 'home' },
  { path: ':id', matcher: customMatcher, component: Users },
  { path: dynamicPath, component: Home },
];
@NgModule({ imports: [RouterModule.forRoot(routes)] })
export class AppModule {}
"#;
    let graph = extract("src/app.routes.ts", source, &OperationContext::default()).unwrap();
    let routes = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["kind"] == "route")
        .map(|node| node["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(routes, ["/home", "/admin/users", "/admin"]);
    let route_refs = graph["unresolvedReferences"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|reference| {
            matches!(
                reference["referenceKind"].as_str(),
                Some("angular_route_component" | "angular_route_layout")
            )
        })
        .map(|reference| reference["referenceName"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(route_refs, ["Home", "Users", "Admin", "AdminHome", "Admin"]);

    let unregistered = r#"import { Routes } from '@angular/router';
const routes: Routes = [{ path: 'fake', component: Fake }];
const tabs = [{ path: 'also-fake', component: Fake }];
"#;
    let graph = extract(
        "unregistered.ts",
        unregistered,
        &OperationContext::default(),
    )
    .unwrap();
    assert!(
        !graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["kind"] == "route")
    );
    assert!(graph["unresolvedReferences"]
        .as_array()
        .unwrap()
        .iter()
        .all(|reference| !matches!(
            reference["referenceKind"].as_str(),
            Some("angular_route_component" | "angular_route_layout")
        )));
    let node_ids = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|node| node["id"].as_str())
        .collect::<std::collections::HashSet<_>>();
    assert!(graph["edges"].as_array().unwrap().iter().all(|edge| {
        [edge["source"].as_str(), edge["target"].as_str()]
            .into_iter()
            .flatten()
            .all(|id| node_ids.contains(id))
    }));

    let ordinary = "// import type { Routes } from '@angular/router';\nconst marker = '@angular/router';\nconst routes = [{ path: 'ordinary', component: View }];";
    let graph = extract("ordinary.ts", ordinary, &OperationContext::default()).unwrap();
    assert!(graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["kind"] == "route"));
}

#[test]
fn for_child_routes_are_marked_relative_and_not_published_as_absolute_paths() {
    let source = r#"import { Route, RouterModule } from '@angular/router';
import { Child } from './child';
const routes: Route[] = [{ path: 'child', component: Child }];
@NgModule({ imports: [RouterModule.forChild(routes)] })
export class ChildModule {}
"#;
    let graph = extract("child.routes.ts", source, &OperationContext::default()).unwrap();
    let route = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "route")
        .unwrap();
    assert_eq!(route["name"], "@forChild/child");
    assert!(
        route["signature"]
            .as_str()
            .unwrap()
            .contains("mount unresolved")
    );
}

#[test]
fn angular_namespaced_router_registration_uses_original_utf16_reference_coordinates() {
    let source = "import * as ar from '@angular/router';\nimport type { Routes } from '@angular/router';\n// 😀\nconst routes: Routes = [{ path: 'home', component: Home }];\nar.RouterModule.forRoot(routes);\n";
    let graph = extract("app.routes.ts", source, &OperationContext::default()).unwrap();
    let reference = graph["unresolvedReferences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|reference| reference["referenceKind"] == "angular_route_component")
        .unwrap();
    let position = source.find("Home").unwrap();
    let prefix = &source[..position];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix.rsplit('\n').next().unwrap().encode_utf16().count();
    assert_eq!(reference["line"], line);
    assert_eq!(reference["column"], column);

    let untrusted_namespace = "import * as ar from '@not-angular/router';\nimport type { Routes } from '@angular/router';\nconst routes: Routes = [{path:'fake', component: Home}];\nar.RouterModule.forRoot(routes);\n";
    let graph = extract(
        "untrusted.routes.ts",
        untrusted_namespace,
        &OperationContext::default(),
    )
    .unwrap();
    assert!(
        !graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["kind"] == "route")
    );
}

#[test]
fn interpolation_only_templates_respect_the_node_budget() {
    let source = "{{ value }}".repeat(5_001);
    let error = extract("large.html", &source, &OperationContext::default()).unwrap_err();
    assert_eq!(error.0, "EXTRACTION_LIMIT");
}

#[test]
fn binding_names_and_ranges_are_original_utf16_coordinates() {
    let source = "😀\r\n<button [title]=\"label\" (click)=\"save()\" #button>Hi</button>";
    let graph = extract("bindings.html", source, &OperationContext::default()).unwrap();
    for name in ["[title]", "(click)", "#button"] {
        let binding = graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["name"] == name)
            .unwrap();
        let byte = source.find(name).unwrap();
        assert_eq!(binding["startLine"], 2);
        assert_eq!(
            binding["startColumn"],
            source[..byte]
                .rsplit('\n')
                .next()
                .unwrap()
                .encode_utf16()
                .count()
        );
    }
}

#[test]
fn raw_script_and_style_text_are_not_misread_as_template_elements() {
    let graph = extract("raw.html", "<script>const text = '<fake>{{ notABinding }}</fake>';</script><STYLE>.x::after { content: '<also-fake>'; }</STYLE><p>real</p>", &OperationContext::default()).unwrap();
    let nodes = graph["nodes"].as_array().unwrap();
    assert!(nodes.iter().any(|node| node["name"] == "p"));
    assert!(!nodes.iter().any(|node| matches!(
        node["name"].as_str(),
        Some("fake" | "also-fake" | "interpolation")
    )));
}

#[test]
fn duplicate_component_decorators_do_not_emit_colliding_graph_ids() {
    let source = "import { Component } from '@angular/core'; @Component({template: ''}) @Component({template: ''}) class Invalid {}";
    assert!(extract("duplicate.ts", source, &OperationContext::default()).is_err());
}

#[test]
fn static_quoted_keys_are_supported_but_spreads_and_type_only_imports_are_not_guessed() {
    let quoted = "import { Component } from '@angular/core'; @Component({'template': '<p>literal</p>'}) class Quoted {}";
    let graph = extract("quoted.ts", quoted, &OperationContext::default()).unwrap();
    assert!(
        graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["name"] == "p")
    );

    let spread = "import { Component } from '@angular/core'; @Component({...defaults, template: '<b>uncertain</b>'}) class Spread {}";
    let graph = extract("spread.ts", spread, &OperationContext::default()).unwrap();
    assert!(
        !graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["name"] == "b")
    );
    assert!(
        graph["coverage"]["notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|notice| notice.as_str().unwrap().contains("spread"))
    );

    for import in [
        "import type { Component } from '@angular/core';",
        "import { type Component } from '@angular/core';",
    ] {
        let source =
            format!("{import} @Component({{template: '<p>not runtime</p>'}}) class TypeOnly {{}}");
        let graph = extract("type-only.ts", &source, &OperationContext::default()).unwrap();
        assert!(
            !graph["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node["kind"] == "component")
        );
    }
}
