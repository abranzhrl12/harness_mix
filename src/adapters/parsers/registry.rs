use super::{astro, csharp, ecmascript, go, python, razor, rust, vue};
use harness_application::ports::parsing::SyntaxParser;
use harness_domain::{Failure, editing, execution::OperationContext, parsing::SourceFormat};
use serde_json::{Value, json};
use std::path::Path;

/// Ordered, dependency-injected parser strategies. The first matching strategy
/// owns validation; errors are authoritative and never trigger fallback.
pub(in crate::adapters) struct ParserRegistry<'a> {
    parsers: Vec<&'a dyn SyntaxParser>,
}

impl<'a> ParserRegistry<'a> {
    pub(in crate::adapters) fn new(parsers: &[&'a dyn SyntaxParser]) -> Self {
        Self {
            parsers: parsers.to_vec(),
        }
    }

    pub(in crate::adapters) fn validate(
        &self,
        path: &str,
        source: &str,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let parser = self
            .parsers
            .iter()
            .copied()
            .find(|parser| parser.supports(path))
            .ok_or((
                "PARSER_UNAVAILABLE",
                "Editing this format is not migrated; original untouched",
            ))?;
        parser.validate(path, source, context)
    }
}

struct RustParser;
impl SyntaxParser for RustParser {
    fn supports(&self, path: &str) -> bool {
        matches!(
            SourceFormat::from_path(Path::new(path)),
            Some(SourceFormat::Rust)
        )
    }

    fn validate(
        &self,
        _path: &str,
        source: &str,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        rust::syntax::with_ast(source, context, |_, _| Ok(()))?;
        Ok(json!({"status":"valid","language":"rust","validation":"syntax_only"}))
    }
}

struct PythonParser;
impl SyntaxParser for PythonParser {
    fn supports(&self, path: &str) -> bool {
        matches!(
            SourceFormat::from_path(Path::new(path)),
            Some(SourceFormat::Python)
        )
    }

    fn validate(
        &self,
        _path: &str,
        source: &str,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        python::syntax::validate_syntax(source, context)?;
        Ok(json!({"status":"valid","language":"python","validation":"syntax_only"}))
    }
}

struct GoParser;
impl SyntaxParser for GoParser {
    fn supports(&self, path: &str) -> bool {
        matches!(
            SourceFormat::from_path(Path::new(path)),
            Some(SourceFormat::Go)
        )
    }

    fn validate(
        &self,
        _path: &str,
        source: &str,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        go::syntax::validate_syntax(source, context)?;
        Ok(json!({"status":"valid","language":"go","validation":"syntax_only"}))
    }
}

struct EcmascriptParser;
impl SyntaxParser for EcmascriptParser {
    fn supports(&self, path: &str) -> bool {
        matches!(
            SourceFormat::from_path(Path::new(path)),
            Some(
                SourceFormat::JavaScript
                    | SourceFormat::JavaScriptModule
                    | SourceFormat::CommonJs
                    | SourceFormat::Jsx
                    | SourceFormat::TypeScript
                    | SourceFormat::TypeScriptModule
                    | SourceFormat::TypeScriptCommonJs
                    | SourceFormat::Tsx
            )
        )
    }

    fn validate(
        &self,
        path: &str,
        source: &str,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        ecmascript::syntax::validate_syntax_with_limit(path, source, editing::MAX_FILE, context)?;
        let language = SourceFormat::from_path(Path::new(path))
            .expect("supports checked the source format")
            .language()
            .protocol_name();
        Ok(json!({"status":"valid","language":language,"validation":"syntax_only"}))
    }
}

struct VueParser;
impl SyntaxParser for VueParser {
    fn supports(&self, path: &str) -> bool {
        matches!(
            SourceFormat::from_path(Path::new(path)),
            Some(SourceFormat::Vue)
        )
    }

    fn validate(
        &self,
        path: &str,
        source: &str,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        vue::syntax::validate(path, source, context)
    }
}

struct AstroParser;
impl SyntaxParser for AstroParser {
    fn supports(&self, path: &str) -> bool {
        matches!(
            SourceFormat::from_path(Path::new(path)),
            Some(SourceFormat::Astro)
        )
    }

    fn validate(
        &self,
        path: &str,
        source: &str,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        astro::syntax::validate(path, source, context)
    }
}

struct CSharpParser;
impl SyntaxParser for CSharpParser {
    fn supports(&self, path: &str) -> bool {
        matches!(
            SourceFormat::from_path(Path::new(path)),
            Some(SourceFormat::CSharp)
        )
    }

    fn validate(
        &self,
        path: &str,
        source: &str,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        csharp::syntax::validate_syntax(path, source, context)?;
        Ok(json!({"status":"valid","language":"csharp","validation":"syntax_only"}))
    }
}

struct RazorParser;
impl SyntaxParser for RazorParser {
    fn supports(&self, path: &str) -> bool {
        matches!(
            SourceFormat::from_path(Path::new(path)),
            Some(SourceFormat::Razor)
        )
    }

    fn validate(
        &self,
        path: &str,
        source: &str,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        razor::syntax::validate(path, source, context)
    }
}

/// Builds the native edit syntax registry. Text and JSON remain outside this
/// registry so edit policy can continue handling them separately.
pub(in crate::adapters) fn default_syntax_registry() -> ParserRegistry<'static> {
    static RUST: RustParser = RustParser;
    static PYTHON: PythonParser = PythonParser;
    static GO: GoParser = GoParser;
    static ECMASCRIPT: EcmascriptParser = EcmascriptParser;
    static VUE: VueParser = VueParser;
    static ASTRO: AstroParser = AstroParser;
    static CSHARP: CSharpParser = CSharpParser;
    static RAZOR: RazorParser = RazorParser;

    ParserRegistry::new(&[
        &RUST,
        &PYTHON,
        &GO,
        &ECMASCRIPT,
        &VUE,
        &ASTRO,
        &CSHARP,
        &RAZOR,
    ])
}
