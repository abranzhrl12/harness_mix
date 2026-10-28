use super::*;

#[test]
fn named_function_in_ready_callback_is_resolvable_without_local_variables() {
    let source = "$(document).ready(function () { const local = 1; function DOrden_BuscarporId(id) { return id; } });";
    let symbols = collect("sample.js", source, "digest", &OperationContext::default()).unwrap();
    let matches: Vec<_> = symbols.iter().filter(|item| item.name == "DOrden_BuscarporId").collect();
    assert_eq!(matches.len(), 1);
    assert!(source[matches[0].bytes.clone()].contains("return id"));
    assert!(!symbols.iter().any(|item| item.name == "local"));
}

#[test]
fn callbacks_with_same_named_functions_remain_distinct() {
    let source = "a(function(){ function same(){return 1;} }); b(function(){ function same(){return 2;} });";
    let symbols = collect("sample.js", source, "digest", &OperationContext::default()).unwrap();
    let matches: Vec<_> = symbols.iter().filter(|item| item.name == "same").collect();
    assert_eq!(matches.len(), 2);
    assert_ne!(matches[0].container, matches[1].container);
    assert_ne!(matches[0].id, matches[1].id);
}

#[test]
fn default_symbol_collection_retains_512_kib_limit() {
    let source = format!(
        "const value = 1;\n//{}",
        "x".repeat(DEFAULT_SOURCE_LIMIT_BYTES),
    );
    let error = collect("sample.js", &source, "digest", &OperationContext::default())
        .err()
        .expect("default parser must reject input larger than 512 KiB");
    assert_eq!(error.0, "PARSER_LIMIT");
    assert!(error.1.contains("512 KiB"));
}
