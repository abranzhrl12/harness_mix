use super::{ParserRegistry, default_syntax_registry};
use harness_application::ports::parsing::SyntaxParser;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};

struct FakeParser {
    extensions: &'static [&'static str],
    result: Result<Value, Failure>,
}

impl SyntaxParser for FakeParser {
    fn supports(&self, path: &str) -> bool {
        let extension = path.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
        self.extensions
            .iter()
            .any(|supported| extension.eq_ignore_ascii_case(supported))
    }

    fn validate(
        &self,
        _path: &str,
        _source: &str,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        context.check()?;
        self.result.clone()
    }
}

#[test]
fn dependency_injected_parser_is_selected_by_extension_case_insensitively() {
    let parser = FakeParser {
        extensions: &["foo", "bar"],
        result: Ok(json!({"parser":"fake"})),
    };
    let registry = ParserRegistry::new(&[&parser]);

    assert_eq!(
        registry
            .validate("src/Thing.FOO", "source", &OperationContext::default())
            .unwrap(),
        json!({"parser":"fake"})
    );
    assert_eq!(
        registry
            .validate("src/Thing.BaR", "source", &OperationContext::default())
            .unwrap(),
        json!({"parser":"fake"})
    );
}

#[test]
fn parser_error_is_returned_without_falling_back() {
    let failing = FakeParser {
        extensions: &["foo"],
        result: Err(("FAKE_ERROR", "selected parser failed")),
    };
    let fallback = FakeParser {
        extensions: &["foo"],
        result: Ok(json!({"fallback":true})),
    };
    let registry = ParserRegistry::new(&[&failing, &fallback]);

    assert_eq!(
        registry
            .validate("file.foo", "source", &OperationContext::default())
            .unwrap_err(),
        ("FAKE_ERROR", "selected parser failed")
    );
}

#[test]
fn unsupported_extension_returns_parser_unavailable() {
    let parser = FakeParser {
        extensions: &["foo"],
        result: Ok(json!({})),
    };
    let registry = ParserRegistry::new(&[&parser]);

    assert_eq!(
        registry
            .validate("file.json", "{}", &OperationContext::default())
            .unwrap_err(),
        (
            "PARSER_UNAVAILABLE",
            "Editing this format is not migrated; original untouched"
        )
    );
}

#[test]
fn cancellation_from_selected_parser_is_preserved() {
    let parser = FakeParser {
        extensions: &["foo"],
        result: Ok(json!({})),
    };
    let registry = ParserRegistry::new(&[&parser]);
    let context = OperationContext::without_deadline();
    context.cancel();

    assert_eq!(
        registry
            .validate("file.foo", "source", &context)
            .unwrap_err()
            .0,
        "CANCELLED"
    );
}

#[test]
fn native_registry_routes_extensions_and_keeps_edit_policy_boundaries() {
    let registry = default_syntax_registry();
    let context = OperationContext::default();
    for path in [
        "x.rs", "x.py", "x.PY", "x.JS", "x.MJS", "x.CJS", "x.JSX", "x.TS", "x.TSX", "x.MTS",
        "x.CTS", "x.VUE", "x.ASTRO", "x.CSHTML", "x.CS",
    ] {
        assert!(registry.validate(path, "", &context).is_ok(), "{path}");
    }
    for path in ["x.json", "x.md", "x.txt", "x.sql", "x.xml", "x.css"] {
        assert_eq!(
            registry.validate(path, "", &context).unwrap_err().0,
            "PARSER_UNAVAILABLE",
            "{path}"
        );
    }
}

#[test]
fn native_csharp_parser_accepts_valid_sources_and_rejects_unbalanced_braces() {
    let registry = default_syntax_registry();
    let context = OperationContext::default();

    let value = registry
        .validate(
            "x.cs",
            "namespace Sample;\npublic class Thing\n{\n    public int Value { get; set; }\n}\n",
            &context,
        )
        .unwrap();
    assert_eq!(value["language"], "csharp");
    assert_eq!(value["status"], "valid");

    assert_eq!(
        registry
            .validate("x.cs", "class Thing { void Method() { }", &context)
            .unwrap_err()
            .0,
        "SYNTAX_ERROR"
    );
    assert_eq!(
        registry
            .validate("x.cs", "class Thing { } }", &context)
            .unwrap_err()
            .0,
        "SYNTAX_ERROR"
    );
}

#[test]
fn native_csharp_parser_ignores_braces_inside_raw_string_literals() {
    let registry = default_syntax_registry();
    let context = OperationContext::default();
    let source = "class Thing\n{\n    string Payload = \"\"\"\n    { not code }\n    \"\"\";\n}\n";

    let value = registry.validate("x.cs", source, &context).unwrap();
    assert_eq!(value["status"], "valid");
}

#[test]
fn native_ecmascript_parsers_accept_language_specific_syntax_and_reject_errors() {
    let registry = default_syntax_registry();
    let context = OperationContext::default();
    for (path, source, language) in [
        ("x.js", "const value = 1;", "javascript"),
        ("x.JS", "const value = 1;", "javascript"),
        ("x.ts", "const value: number = 1;", "typescript"),
        ("x.TS", "const value: number = 1;", "typescript"),
        ("x.jsx", "const element = <div />;", "javascript"),
        ("x.JSX", "const element = <div />;", "javascript"),
        (
            "x.tsx",
            "const element: JSX.Element = <div />;",
            "typescript",
        ),
        (
            "x.TSX",
            "const element: JSX.Element = <div />;",
            "typescript",
        ),
    ] {
        let result = registry.validate(path, source, &context).unwrap();
        assert_eq!(result["language"], language, "{path}");
    }

    for path in ["x.js", "x.ts", "x.jsx", "x.tsx", "x.TS", "x.TSX"] {
        assert_eq!(
            registry
                .validate(path, "const value: number = ;", &context)
                .unwrap_err()
                .0,
            "SYNTAX_ERROR",
            "{path}"
        );
    }
}

#[test]
fn native_rust_parser_returns_the_existing_validation_shape() {
    let value = default_syntax_registry()
        .validate("x.rs", "fn main() {}", &OperationContext::default())
        .unwrap();
    assert_eq!(
        value,
        json!({"status":"valid","language":"rust","validation":"syntax_only"})
    );
}

#[test]
fn native_python_parser_accepts_valid_source_and_rejects_syntax_errors() {
    let registry = default_syntax_registry();
    let context = OperationContext::default();
    let value = registry
        .validate("x.py", "class Foo:\n    def bar(self, x: int) -> int:\n        return x + 1\n", &context)
        .unwrap();
    assert_eq!(
        value,
        json!({"status":"valid","language":"python","validation":"syntax_only"})
    );
    assert_eq!(
        registry
            .validate("x.py", "def broken(:\n    pass\n", &context)
            .unwrap_err()
            .0,
        "SYNTAX_ERROR"
    );
}
