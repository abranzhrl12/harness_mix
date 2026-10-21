use agent_harness::adapters::index_extract::extract;
use harness_domain::execution::OperationContext;

fn graph(source: &str) -> serde_json::Value {
    extract("src/providers.ts", source, &OperationContext::default()).unwrap()
}

#[test]
fn imported_use_class_and_use_existing_emit_separate_registration_relationships() {
    let result = graph(
        "import { NgModule } from '@angular/core';\nimport { PORT as ApiPort, PORT2, ApiImpl, LegacyApi } from './api';\n@NgModule({ providers: [{ provide: ApiPort, useClass: ApiImpl }, { provide: PORT2, useExisting: LegacyApi }] }) export class AppModule {}",
    );
    let nodes = result["nodes"].as_array().unwrap();
    let registrations = nodes
        .iter()
        .filter(|node| {
            node["kind"] == "module"
                && node["signature"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("Angular provider registration ")
        })
        .collect::<Vec<_>>();
    assert_eq!(registrations.len(), 2);
    assert!(registrations.iter().all(|node| {
        node["id"]
            .as_str()
            .unwrap()
            .starts_with("native:framework:")
    }));
    assert!(registrations.iter().all(|node| {
        node["signature"]
            .as_str()
            .unwrap()
            .contains("scope=AppModule")
    }));
    assert_eq!(
        result["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|edge| edge["kind"] == "contains"
                && registrations
                    .iter()
                    .any(|node| edge["target"] == node["id"]))
            .count(),
        2
    );
    let refs = result["unresolvedReferences"].as_array().unwrap();
    assert_eq!(
        refs.iter()
            .filter(|reference| reference["referenceKind"] == "angular_provider_token")
            .count(),
        2
    );
    assert_eq!(
        refs.iter()
            .filter(|reference| reference["referenceKind"] == "angular_provider_implementation")
            .count(),
        2
    );
    assert!(refs.iter().any(
        |reference| reference["referenceKind"] == "angular_provider_token"
            && reference["metadata"]["imported"] == "PORT"
    ));
    assert!(refs.iter().any(|reference| reference["referenceKind"]
        == "angular_provider_implementation"
        && reference["metadata"]["registrationForm"] == "useExisting"));
    assert_eq!(
        result["coverage"]["providers"]["runtimeInjectorSelectionEvaluated"],
        false
    );
}

#[test]
fn rejects_duplicate_multi_dynamic_factory_and_non_imported_provider_values() {
    let result = graph(
        "import { Component } from '@angular/core';\nimport { PORT, Impl } from './api';\nconst choices = [];\n@Component({ providers: [{provide: PORT, useClass: Impl}, {provide: PORT, useExisting: Impl}, {provide: PORT, useClass: Impl, multi: true}, {provide: PORT, useFactory: make}, ...choices, {provide: localToken, useClass: Impl}] }) class Card {}",
    );
    assert!(result["nodes"].as_array().unwrap().iter().all(|node| {
        !node["signature"]
            .as_str()
            .unwrap_or("")
            .starts_with("Angular provider registration ")
    }));
    assert!(
        result["unresolvedReferences"]
            .as_array()
            .unwrap()
            .iter()
            .all(|reference| !reference["referenceKind"]
                .as_str()
                .unwrap_or("")
                .starts_with("angular_provider_"))
    );
    let notices = result["coverage"]["providers"]["notices"]
        .as_array()
        .unwrap();
    assert!(
        notices
            .iter()
            .any(|notice| notice.as_str().unwrap().contains("spread"))
    );

    for (entry, expected_notice) in [
        ("{provide: PORT, useClass: Impl, multi: true}", "multi"),
        ("{provide: PORT, useFactory: make}", "factory"),
        ("{provide: localToken, useClass: Impl}", "non-imported"),
    ] {
        let source = format!(
            "import {{ Component }} from '@angular/core'; import {{ PORT, Impl }} from './api'; @Component({{providers:[{entry}]}}) class Card {{}}"
        );
        let result = graph(&source);
        assert!(result["nodes"].as_array().unwrap().iter().all(|node| {
            !node["signature"]
                .as_str()
                .unwrap_or("")
                .starts_with("Angular provider registration ")
        }));
        assert!(
            result["coverage"]["providers"]["notices"]
                .as_array()
                .unwrap()
                .iter()
                .any(|notice| notice.as_str().unwrap().contains(expected_notice))
        );
    }
    let duplicates = graph(
        "import { Component } from '@angular/core'; import { PORT, Impl } from './api'; @Component({providers:[{provide: PORT, useClass: Impl}, {provide: PORT, useExisting: Impl}]}) class Card {}",
    );
    assert!(
        duplicates["coverage"]["providers"]["notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|notice| notice.as_str().unwrap().contains("same static token"))
    );
}

#[test]
fn accepts_namespace_core_decorator_and_does_not_treat_local_decorator_as_angular() {
    let valid = graph(
        "import * as ng from '@angular/core';\nimport { TOKEN, Impl } from './api';\n@ng.Component({providers: [{provide: TOKEN, useClass: Impl}]}) class Card {}",
    );
    assert_eq!(valid["coverage"]["providers"]["registrationsEmitted"], 1);
    let invalid = graph(
        "import { TOKEN, Impl } from './api';\nfunction Component(x: unknown) {}\n@Component({providers: [{provide: TOKEN, useClass: Impl}]}) class Card {}",
    );
    assert!(invalid["nodes"].as_array().unwrap().iter().all(|node| {
        !node["signature"]
            .as_str()
            .unwrap_or("")
            .starts_with("Angular provider registration ")
    }));
}

#[test]
fn type_only_decorators_tokens_and_implementations_do_not_prove_registrations() {
    for source in [
        "import type * as ng from '@angular/core'; import { TOKEN, Impl } from './api'; @ng.Component({providers:[{provide:TOKEN,useClass:Impl}]}) class Card {}",
        "import type { Component } from '@angular/core'; import { TOKEN, Impl } from './api'; @Component({providers:[{provide:TOKEN,useClass:Impl}]}) class Card {}",
        "import { Component } from '@angular/core'; import type { TOKEN } from './api'; import { Impl } from './api'; @Component({providers:[{provide:TOKEN,useClass:Impl}]}) class Card {}",
        "import { Component } from '@angular/core'; import { TOKEN, type Impl } from './api'; @Component({providers:[{provide:TOKEN,useClass:Impl}]}) class Card {}",
    ] {
        let result = graph(source);
        assert!(
            result["unresolvedReferences"]
                .as_array()
                .unwrap()
                .iter()
                .all(|reference| {
                    !reference["referenceKind"]
                        .as_str()
                        .unwrap_or("")
                        .starts_with("angular_provider_")
                }),
            "{source}"
        );
    }
}
