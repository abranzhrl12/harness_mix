use super::csharp::blank_csharp;
use super::decode::decode;
use super::orchestration::extract;
use codegraph_native::buffers::*;
use harness_domain::execution::OperationContext;

#[test]
fn preprocess_preserves_utf16_columns_crlf_and_both_branches() {
    let input = "  #if FLAG // 😀\r\nclass A {}\r\n#else\r\nclass B {}\r\n#endif\r\n#region hi\r\n";
    let output = blank_csharp(input).unwrap();
    assert_eq!(input.encode_utf16().count(), output.encode_utf16().count());
    assert!(output.contains("class A {}\r\n"));
    assert!(output.contains("class B {}\r\n"));
    assert!(!output.contains("#if"));
    assert!(output.contains("#region hi"));
    assert_eq!(blank_csharp("#iffy\n").unwrap(), "#iffy\n");
}
#[test]
fn malformed_buffers_and_cancellation_fail_closed() {
    let raw = EmitOut {
        meta: vec![],
        nodes: vec![],
        edges: vec![],
        refs: vec![],
        arena: vec![],
    };
    assert!(decode(&raw, "x.rs", "rust", &OperationContext::default()).is_err());
    let context = OperationContext::default();
    context.cancel();
    assert_eq!(
        extract("x.rs", "fn x() {}", &context).unwrap_err().0,
        "CANCELLED"
    );
}
