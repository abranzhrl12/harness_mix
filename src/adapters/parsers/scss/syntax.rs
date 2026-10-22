//! SCSS syntax validation with real positions.
//!
//! Validation is syntactic only: module resolution (`@use`, `@forward`) and
//! value semantics are never evaluated, so a valid file never fails because an
//! import cannot be located.
use harness_domain::{Failure, execution::OperationContext};
use raffia::{Parser, Syntax, ast::Stylesheet, error::Error};
use serde_json::{Value, json};

/// A SCSS diagnostic with its one-based position.
pub(in crate::adapters) struct Diagnostic {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

/// Validates `source` as SCSS.
///
/// Recoverable errors are reported as warnings and never block an edit.
pub(in crate::adapters) fn validate(
    source: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let mut parser = Parser::new(source, Syntax::Scss);
    match parser.parse::<Stylesheet>() {
        Ok(_) => {
            let mut result =
                json!({"status":"valid","language":"scss","validation":"syntax_only"});
            let recovered = diagnostics(source, parser.recoverable_errors());
            if !recovered.is_empty() {
                result["status"] = json!("valid_with_warnings");
                result["warnings"] = json!(recovered
                    .iter()
                    .map(|diagnostic| json!({
                        "code":"SCSS_RECOVERABLE",
                        "message":diagnostic.message,
                        "line":diagnostic.line,
                        "column":diagnostic.column
                    }))
                    .collect::<Vec<_>>());
            }
            Ok(result)
        }
        Err(_) => Err(("SYNTAX_ERROR", "SCSS source is not syntactically valid")),
    }
}

/// First hard error of `source`, with its position.
pub(in crate::adapters) fn first_error(source: &str) -> Option<Diagnostic> {
    let mut parser = Parser::new(source, Syntax::Scss);
    match parser.parse::<Stylesheet>() {
        Ok(_) => None,
        Err(error) => Some(diagnostic(source, &error)),
    }
}

/// Diagnostics of the errors a parser recovered from.
fn diagnostics(source: &str, errors: &[Error]) -> Vec<Diagnostic> {
    errors
        .iter()
        .map(|error| diagnostic(source, error))
        .collect()
}

fn diagnostic(source: &str, error: &Error) -> Diagnostic {
    let (line, column) = position(source, error.span.start);
    Diagnostic {
        message: error.kind.to_string(),
        line,
        column,
    }
}

/// One-based line and column of a byte offset.
pub(in crate::adapters) fn position(source: &str, at: usize) -> (usize, usize) {
    let at = at.min(source.len());
    let prefix = &source[..at];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .encode_utf16()
        .count()
        + 1;
    (line, column)
}
