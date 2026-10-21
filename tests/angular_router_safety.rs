use agent_harness::adapters::index_extract::extract;
use harness_domain::execution::OperationContext;

fn route_count(source: &str) -> usize {
    let graph = extract("app.routes.ts", source, &OperationContext::default()).unwrap();
    graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["kind"] == "route")
        .count()
}

#[test]
fn escaped_identifier_mutation_does_not_certify_a_static_route_table() {
    let source = r#"
import type { Routes } from '@angular/router';
import { provideRouter } from '@angular/router';
import { Home } from './home';
const routes: Routes = [{ path: 'home', component: Home }];
rou\u0074es.push({ path: 'dynamic', component: Home });
provideRouter(routes);
"#;
    assert_eq!(route_count(source), 0);
}

#[test]
fn comments_and_unrelated_scopes_do_not_change_static_binding_identity() {
    let source = r#"
import type { Routes } from '@angular/router';
import { provideRouter } from '@angular/router';
import { Home } from './home';
// routes is the name of the outer table, not a third use.
function unrelated(routes: string) { return routes; }
const routes: Routes = [{ path: 'home', component: Home }];
provideRouter(routes);
"#;
    assert_eq!(route_count(source), 1);
}

#[test]
fn empty_path_layout_preserves_a_single_leading_slash() {
    let source = r#"
import { provideRouter } from '@angular/router';
import { Home, Layout } from './home';
provideRouter([{path: '', component: Layout, children: [
  {path: 'home', component: Home}
]}]);
"#;
    let graph = extract("app.routes.ts", source, &OperationContext::default()).unwrap();
    let names = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["kind"] == "route")
        .map(|node| node["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"/home"));
    assert!(!names.iter().any(|name| name.starts_with("//")));
}
