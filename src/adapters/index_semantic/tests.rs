use super::*;

#[test]
fn legacy_datatables_helper_can_be_analyzed() {
    let source = include_str!("../../../tests/fixtures/datatables-helper.js");
    let result = analyze(source, "datatables-helper.js", &OperationContext::default());
    assert!(result.is_ok(), "{:?}", result.err());
}
#[test]
fn oxc_type_exports_are_not_runtime_star_shadows() {
    let context = OperationContext::default();
    let typed = analyze("export type * from './types';", "barrel.ts", &context).unwrap();
    assert!(typed.stars.is_empty());
    assert!(typed.exports.is_empty());
    let mixed = analyze(
        "export type {Api as api} from './types'; export * from './leaf';",
        "barrel.ts",
        &context,
    )
    .unwrap();
    assert!(!mixed.exports.contains_key("api"));
    assert_eq!(mixed.stars, ["./leaf"]);
}
#[test]
fn lexical_scopes_keep_import_call_but_not_shadowed_calls() {
    let source = "import {run as execute} from './lib';\nfunction local(execute: () => void) { execute(); }\nexport function caller() { execute(); }";
    let calls = analyze(source, "app.ts", &OperationContext::default()).unwrap();
    assert_eq!(calls.sites.len(), 1);
    assert!(calls.contains("execute", 3, 27));
}

#[test]
fn unrelated_legacy_var_redeclarations_do_not_block_import_analysis() {
    let source = "import {run as execute} from './lib'; var legacy = 1; var legacy = 2; export function caller() { execute(); }";
    let calls = analyze(source, "app.js", &OperationContext::default()).unwrap();
    assert_eq!(calls.sites.len(), 1);
    assert!(calls.sites.iter().any(|(name, _, _)| name == "execute"));
}
#[test]
fn semantic_positions_match_codegraph_with_unicode_and_crlf() {
    let source = "import {run as execute} from './lib';\r\nexport function caller() { const emoji = '😀'; execute(); }\r\n";
    let context = OperationContext::default();
    let calls = analyze(source, "app.ts", &context).unwrap();
    let graph = super::super::index_extract::extract("app.ts", source, &context).unwrap();
    let reference = graph["unresolvedReferences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["referenceKind"] == "calls" && r["referenceName"] == "execute")
        .unwrap();
    assert!(calls.contains(
        "execute",
        reference["line"].as_i64().unwrap(),
        reference["column"].as_i64().unwrap()
    ));
    assert_eq!(calls.sites.len(), 1);
}
#[test]
fn callbacks_members_and_type_only_imports_are_not_direct_import_calls() {
    for source in [
        "import {run as execute} from './lib'; export function caller() { register(execute); this.execute(); }",
        "import type {run as execute} from './lib'; export function caller() { execute(); }",
    ] {
        assert!(
            analyze(source, "app.ts", &OperationContext::default())
                .unwrap()
                .sites
                .is_empty()
        );
    }
}
#[test]
fn invalid_conflicting_cancelled_and_oversized_sources_are_rejected() {
    let context = OperationContext::default();
    assert!(analyze("function {", "app.ts", &context).is_err());
    assert!(
        analyze(
            "import {run as execute} from './lib'; const execute = () => {}; execute();",
            "app.ts",
            &context
        )
        .is_err()
    );
    assert_eq!(
        analyze(&" ".repeat(1024 * 1024 + 1), "app.ts", &context)
            .err()
            .unwrap()
            .0,
        "SYNC_LIMIT"
    );
    context.cancel();
    assert_eq!(
        analyze("", "app.ts", &context).err().unwrap().0,
        "CANCELLED"
    );
}

#[test]
fn runtime_exports_are_separate_from_legacy_function_policy() {
    let facts = analyze("export class Api {} export const useApi = () => 1; export let mutable = () => 2; export const wrapped = factory(() => 3);", "api.ts", &OperationContext::default()).unwrap();
    assert_eq!(facts.exports.get("Api"), Some(&Export::Unsupported));
    assert_eq!(facts.exports.get("useApi"), Some(&Export::Unsupported));
    assert_eq!(
        facts.runtime_exports.get("Api"),
        Some(&Export::Local("Api".into()))
    );
    assert_eq!(
        facts.runtime_exports.get("useApi"),
        Some(&Export::Local("useApi".into()))
    );
    assert_eq!(facts.runtime_kinds.get("Api"), Some(&RuntimeKind::Class));
    assert_eq!(
        facts.runtime_kinds.get("useApi"),
        Some(&RuntimeKind::Function)
    );
    assert_eq!(
        facts.runtime_exports.get("mutable"),
        Some(&Export::Unsupported)
    );
    assert_eq!(
        facts.runtime_exports.get("wrapped"),
        Some(&Export::Unsupported)
    );
}
