use ra_syntax_compare::{baseline, candidate};

fn snapshot(source: &str) -> (serde_json::Value, serde_json::Value) {
    let syn = baseline(source).unwrap();
    let ra = candidate::collect(source).unwrap();
    (ra_syntax_compare::snapshot(&syn), ra_syntax_compare::snapshot(&ra))
}

#[test]
fn declaration_contract_matches_production_extractor() {
    for source in [
        "pub fn simple(a: u8) -> u8 { a }",
        "/// docs\r\n#[allow(dead_code)]\r\npub struct Data { x:u8 }\r\n",
        "mod inner { enum E { A } type T = u8; const C: u8 = 1; static S: u8 = 2; }",
        "trait T { fn required(&self); fn defaulted(&self) {} const C:u8; type A; }\nimpl T for Data { fn required(&self) {} const C:u8=1; type A=u8; }",
        "\u{feff}#!/usr/bin/env rustx\r\n/// 😀\r\nfn unicode() { let s = \"😀\"; }\r\n",
        "fn outer() { fn hidden() {} } mod external; struct Pair(u8,u8);",
        "#[doc = \"escaped\\ntext\"] fn documented() {}",
        "macro_rules! make { () => { fn generated() {} } }",
        "unsafe impl !Send for Data {} union U { x:u32, y:f32 } trait Alias = Send;",
        "// ordinary comment\nfn r#type() {}\n/** block\ndocs */ fn block_doc() {}",
    ] {
        let (syn, ra) = snapshot(source);
        assert_eq!(syn, ra, "source: {source}");
    }
}

#[test]
fn strict_validation_rejects_invalid_and_large_sources() {
    for source in ["fn bad( {", "fn okay() {}\nfn unfinished("] {
        assert!(baseline(source).is_err());
        assert!(candidate::collect(source).is_err());
    }
    let large = " ".repeat(256 * 1024 + 1);
    assert_eq!(baseline(&large).err().unwrap().0, "PARSER_LIMIT");
    assert_eq!(candidate::collect(&large).err().unwrap().0, "PARSER_LIMIT");
    let deep = format!("fn f() {{ {}0{} }}", "(".repeat(70), ")".repeat(70));
    assert_eq!(baseline(&deep).err().unwrap().0, "PARSER_LIMIT");
    assert_eq!(candidate::collect(&deep).err().unwrap().0, "PARSER_LIMIT");
}

#[test]
fn real_repository_symbols_match() {
    for (path, source) in [
        ("rust/syntax.rs", include_str!("../../../src/adapters/parsers/rust/syntax.rs")),
        ("rust/symbols.rs", include_str!("../../../src/adapters/parsers/rust/symbols.rs")),
        ("index_rust.rs", include_str!("../../../src/adapters/index_rust.rs")),
    ] {
        let (syn, ra) = snapshot(source);
        let differences = syn.as_array().unwrap().iter().zip(ra.as_array().unwrap())
            .filter(|(a,b)| a != b).count();
        println!("{path}: syn={}, ra={}, differing_pairs={differences}",
            syn.as_array().unwrap().len(), ra.as_array().unwrap().len());
        assert_eq!(syn, ra, "real repository contract: {path}");
    }
}
