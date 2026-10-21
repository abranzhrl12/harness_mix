use agent_harness::adapters::index_extract::extract;
use harness_domain::execution::OperationContext;
use serde_json::Value;

fn di_refs(source: &str) -> Vec<Value> {
    let graph = extract("src/app.ts", source, &OperationContext::default()).unwrap();
    graph["unresolvedReferences"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|reference| reference["referenceKind"] == "angular_injects")
        .cloned()
        .collect()
}

#[test]
fn constructor_type_import_records_a_proven_di_token() {
    let refs = di_refs(
        "import { Component } from '@angular/core';\nimport { Dep } from './dep';\n@Component({}) export class App { constructor(dep: Dep) {} }",
    );
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0]["referenceName"], "Dep");
    assert_eq!(refs[0]["referenceKind"], "angular_injects");
    assert_eq!(refs[0]["metadata"]["module"], "./dep");
    assert_eq!(refs[0]["metadata"]["imported"], "Dep");
    assert_eq!(refs[0]["metadata"]["tokenForm"], "constructor_type");
    assert_eq!(refs[0]["line"], 3);
}

#[test]
fn explicit_inject_override_and_imported_inject_calls_keep_their_forms() {
    let refs = di_refs(
        "import { Component, Inject, inject as get } from '@angular/core';\nimport { Dep, ALT as Alias } from './dep';\n@Component({}) export class App { constructor(@Inject(Alias) dep: Dep) {} field = get(Dep); constructor2() { get(Alias); } }",
    );
    assert_eq!(refs.len(), 2);
    assert!(refs.iter().any(|reference| {
        reference["referenceName"] == "Alias"
            && reference["metadata"]["imported"] == "ALT"
            && reference["metadata"]["tokenForm"] == "inject_decorator"
    }));
    assert!(refs.iter().any(|reference| {
        reference["referenceName"] == "Dep" && reference["metadata"]["tokenForm"] == "inject_call"
    }));
}

#[test]
fn lexical_shadowing_and_deferred_callbacks_do_not_create_di_proofs() {
    let refs = di_refs(
        "import { Component, inject } from '@angular/core';\nimport { Dep } from './dep';\n@Component({}) export class App { field = inject(Dep); constructor() { const later = () => inject(Dep); function nested() { inject(Dep); } } method() { inject(Dep); } }\n@Component({}) class Shadow { constructor(Dep: string) { const inject = (x: unknown) => x; inject(Dep); } }",
    );
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0]["referenceName"], "Dep");
    assert_eq!(refs[0]["metadata"]["tokenForm"], "inject_call");
}

#[test]
fn type_only_dynamic_and_unimported_tokens_stay_pending() {
    let refs = di_refs(
        "import { Component, Inject } from '@angular/core';\nimport type { Dep } from './dep';\nconst TOKEN = 'token';\n@Component({}) export class App { constructor(dep: Dep, @Inject(TOKEN) other: Dep) {} }\nfunction Component() {} @Component({}) class Local {}",
    );
    assert!(refs.is_empty());
}

#[test]
fn dynamic_inject_override_does_not_fall_back_to_an_imported_parameter_type() {
    let refs = di_refs(
        "import { Component, Inject } from '@angular/core';\nimport { Dep } from './dep';\nconst TOKEN = Symbol('dep');\n@Component({}) export class App { constructor(@Inject(TOKEN) dep: Dep) {} }",
    );
    assert!(refs.is_empty());
}

#[test]
fn static_fields_and_deferred_initializers_are_not_constructor_contexts() {
    let refs = di_refs(
        "import { Component, inject } from '@angular/core'; import { Dep } from './dep'; @Component({}) class App { static value = inject(Dep); deferred = () => inject(Dep); constructor() { inject(Dep); } }",
    );
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0]["metadata"]["tokenForm"], "inject_call");
}

#[test]
fn type_only_tokens_do_not_prove_runtime_dependencies() {
    for import in [
        "import type { Dep } from './dep';",
        "import { type Dep } from './dep';",
    ] {
        let source = format!(
            "import {{ Component }} from '@angular/core'; {import} @Component({{}}) class App {{ constructor(dep: Dep) {{}} }}"
        );
        assert!(di_refs(&source).is_empty());
    }
}

#[test]
fn uppercase_typescript_extensions_keep_di_analysis() {
    let source = "import { Component } from '@angular/core'; import { Dep } from './dep'; @Component({}) class App { constructor(dep: Dep) {} }";
    let graph = extract("App.TS", source, &OperationContext::default()).unwrap();
    assert!(
        graph["unresolvedReferences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["referenceKind"] == "angular_injects")
    );
}
