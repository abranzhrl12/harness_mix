//! Actual production RA collector versus the frozen pre-migration syn implementation.
#[path = "../../../../../experiments/ra-syntax-compare/baseline/syntax.rs"]
mod syntax;
#[path = "../../../../../experiments/ra-syntax-compare/baseline/symbols.rs"]
mod old_symbols;
use harness_domain::{execution::OperationContext, hash, parsing::Symbol};
use serde_json::{json, Value};

fn snapshot(symbols: &[Symbol]) -> Value {
    json!(symbols.iter().map(|s| json!({"symbol":s,"from":s.bytes.start,"to":s.bytes.end})).collect::<Vec<_>>())
}

#[test]
fn rust_ra_production_symbol_contract_matches_frozen_syn() {
    for source in [
        "pub fn simple(a:u8) -> u8 { a }",
        "/// docs\r\n#[allow(dead_code)]\r\npub struct Data { x:u8 }\r\n",
        "mod inner { enum E { A } type T=u8; const C:u8=1; static S:u8=2; }",
        "trait T { fn required(&self); fn defaulted(&self) {} const C:u8; type A; } impl T for Data { fn required(&self) {} const C:u8=1; type A=u8; }",
        "\u{feff}#!/usr/bin/env rustx\r\n/// 😀\r\nfn unicode() { let s=\"😀\"; }\r\n",
        "fn outer() { fn hidden() {} } mod external; struct Pair(u8,u8);",
        "#[doc = \"escaped\\ntext\"] fn documented() {}",
        "macro_rules! make { () => { fn generated() {} } }",
        "unsafe impl !Send for Data {} union U { x:u32, y:f32 } trait Alias = Send;",
        "// normal comment\nfn r#type() {}\n/** block\ndocs */ fn block_doc() {}",
        "#[docx = \"not docs\"] fn unrelated_attr() {}",
    ] {
        let context = OperationContext::default();
        let digest = hash(source.as_bytes());
        let before = old_symbols::collect(source, &digest, &context).unwrap();
        let after = super::symbols::collect(source, &digest, &context).unwrap();
        assert_eq!(snapshot(&before), snapshot(&after), "{source}");
    }
}

#[test]
fn rust_ra_validation_keeps_size_depth_and_cancellation_guards() {
    let context = OperationContext::default();
    context.cancel();
    assert_eq!(super::syntax::with_ast("fn f() {}", &context, |_,_| Ok(())).err().unwrap().0, "CANCELLED");
    for (source, expected) in [
        ("fn bad( {".to_owned(), "SYNTAX_ERROR"),
        (" ".repeat(256 * 1024 + 1), "PARSER_LIMIT"),
        (format!("fn f() {{ {}0{} }}", "(".repeat(70), ")".repeat(70)), "PARSER_LIMIT"),
    ] {
        assert_eq!(super::syntax::with_ast(&source, &OperationContext::default(), |_,_| Ok(())).err().unwrap().0, expected);
    }
}
