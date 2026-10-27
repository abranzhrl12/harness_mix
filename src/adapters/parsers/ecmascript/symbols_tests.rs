use super::*;

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
