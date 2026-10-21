//! JavaScript and TypeScript AST syntax validation using Oxc parser.
//! Guarantees syntax safety before edits are committed in harness_replace.

use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

pub(in crate::adapters) fn validate_syntax(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<(), Failure> {
    context.check()?;
    if source.len() > 512 * 1024 {
        return Err(("PARSER_LIMIT", "JS/TS parser limited to 512 KiB"));
    }

    // Oxc's path inference is case-sensitive on some platforms. Normalize only
    // the path used for syntax selection; source text and parser spans stay intact.
    let normalized_path = path.to_ascii_lowercase();
    let source_type = SourceType::from_path(normalized_path.as_str()).unwrap_or_default();
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();

    context.check()?;
    if !parsed.errors.is_empty() {
        return Err((
            "SYNTAX_ERROR",
            "JavaScript/TypeScript source is not syntactically valid",
        ));
    }

    Ok(())
}

/// Reparse only after a rejected candidate to report the first parser span.
/// The normal validation path keeps its existing failure contract.
pub(in crate::adapters) fn first_error_location(
    path: &str,
    source: &str,
) -> Option<(usize, usize)> {
    let normalized_path = path.to_ascii_lowercase();
    let source_type = SourceType::from_path(normalized_path.as_str()).unwrap_or_default();
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    let error = parsed.errors.first()?;
    let offset = error
        .labels
        .as_ref()
        .and_then(|labels| labels.first())
        .map(|label| label.offset())?;
    let prefix = source.get(..offset.min(source.len()))?;
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let last_line = prefix.rsplit('\n').next()?;
    let last_line = last_line.strip_suffix('\r').unwrap_or(last_line);
    // Report editor-style one-based columns in UTF-16 code units, matching JS tooling.
    let column = last_line.encode_utf16().count() + 1;
    Some((line, column))
}

#[cfg(test)]
#[path = "syntax_tests.rs"]
mod tests;
