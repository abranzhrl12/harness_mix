use harness_domain::parsing::{SourceFormat, SourceLanguage, Symbol};
use serde_json::json;
use std::path::Path;

#[test]
fn classifies_supported_extensions_without_losing_dialects() {
    let cases = [
        ("a.RS", SourceFormat::Rust, SourceLanguage::Rust),
        ("a.js", SourceFormat::JavaScript, SourceLanguage::JavaScript),
        (
            "a.MJS",
            SourceFormat::JavaScriptModule,
            SourceLanguage::JavaScript,
        ),
        ("a.cjs", SourceFormat::CommonJs, SourceLanguage::JavaScript),
        ("a.jsx", SourceFormat::Jsx, SourceLanguage::JavaScript),
        ("a.ts", SourceFormat::TypeScript, SourceLanguage::TypeScript),
        (
            "a.mts",
            SourceFormat::TypeScriptModule,
            SourceLanguage::TypeScript,
        ),
        (
            "a.cts",
            SourceFormat::TypeScriptCommonJs,
            SourceLanguage::TypeScript,
        ),
        ("a.tsx", SourceFormat::Tsx, SourceLanguage::TypeScript),
        ("a.vue", SourceFormat::Vue, SourceLanguage::Vue),
        ("a.astro", SourceFormat::Astro, SourceLanguage::Astro),
        ("a.cs", SourceFormat::CSharp, SourceLanguage::CSharp),
        ("a.cshtml", SourceFormat::Razor, SourceLanguage::Razor),
        ("a.CSS", SourceFormat::Css, SourceLanguage::Css),
        ("a.html", SourceFormat::Html, SourceLanguage::Html),
        ("a.HTM", SourceFormat::Html, SourceLanguage::Html),
    ];

    for (path, format, language) in cases {
        assert_eq!(
            SourceFormat::from_path(Path::new(path)),
            Some(format),
            "{path}"
        );
        assert_eq!(format.language(), language, "{path}");
    }
    assert_eq!(SourceFormat::Jsx.language().protocol_name(), "javascript");
    assert_eq!(SourceFormat::Tsx.language().protocol_name(), "typescript");
}

#[test]
fn unknown_or_missing_extensions_are_not_classified() {
    for path in ["README", "file.txt", "file.rs.bak", ".hidden"] {
        assert_eq!(SourceFormat::from_path(Path::new(path)), None, "{path}");
    }
}

#[test]
fn symbol_keeps_the_existing_serialized_shape() {
    let symbol = Symbol {
        name: "item".into(),
        container: "module".into(),
        kind: "function",
        start: 1,
        end: 2,
        signature: "fn item()".into(),
        attributes: Vec::new(),
        doc: String::new(),
        id: "id-1".into(),
        bytes: 4..12,
    };
    assert_eq!(
        serde_json::to_value(symbol).unwrap(),
        json!({
            "name": "item",
            "container": "module",
            "kind": "function",
            "start": 1,
            "end": 2,
            "signature": "fn item()",
            "symbolId": "id-1"
        })
    );
}
