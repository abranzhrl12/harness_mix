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
