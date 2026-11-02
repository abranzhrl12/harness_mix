//! Strict RA syntax boundary shared by navigation, validation and call proofs.
use harness_domain::{Failure, execution::OperationContext};

use ra_ap_syntax::{Edition, SourceFile};
use crate::adapters::index_profile;

/// Syntax-only check. Line endings are normalised first: the RA parser reports
/// errors for some valid constructs when every line ends in CRLF, as in Windows
/// checkouts, and validation never needs byte offsets into the original text.
pub(in crate::adapters) fn validate(
    source: &str,
    context: &OperationContext,
) -> Result<(), Failure> {
    if source.contains("\r\n") {
        with_ast(&source.replace("\r\n", "\n"), context, |_, _| Ok(()))
    } else {
        with_ast(source, context, |_, _| Ok(()))
    }
}

/// Callers return owned data; parsing remains strict even though RA can recover.
pub(in crate::adapters) fn with_ast<T>(
    source: &str,
    context: &OperationContext,
    consume: impl FnOnce(&SourceFile, usize) -> Result<T, Failure>,
) -> Result<T, Failure> {
    context.check()?;
    if source.len() > 256 * 1024 {
        return Err(("PARSER_LIMIT", "Rust parser limited to 256 KiB"));
    }
    let mut depth = 0usize;
    for bytes in source.as_bytes().chunks(32768) {
        context.check()?;
        for byte in bytes {
            if matches!(byte, b'(' | b'{' | b'[') {
                depth += 1;
            } else if matches!(byte, b')' | b'}' | b']') {
                depth = depth.saturating_sub(1);
            }
            if depth > 64 {
                return Err(("PARSER_LIMIT", "Conservative Rust nesting budget exceeded"));
            }
        }
    }
    let base = if source.starts_with('\u{feff}') { 3 } else { 0 };
    let input = &source[base..];
    let prepared = if input.starts_with("#!") && !input.starts_with("#![") {
        let end = input.find('\n').unwrap_or(input.len());
        format!("{}{}", " ".repeat(end), &input[end..])
    } else {
        input.to_owned()
    };
    let parsed = {
        let _timer = index_profile::operation("rust_parse");
        SourceFile::parse(&prepared, Edition::Edition2024)
    };
    context.check()?;
    // No Cargo edition is supplied by this interface. Preserve legacy syntax
    // such as `fn gen()` instead of imposing 2024's new reserved keywords.
    let parsed = if parsed.errors().is_empty() {
        parsed
    } else {
        context.check()?;
        let legacy = {
            let _timer = index_profile::operation("rust_parse_legacy");
            SourceFile::parse(&prepared, Edition::Edition2021)
        };
        context.check()?;
        if !legacy.errors().is_empty() {
            return Err(("SYNTAX_ERROR", "Rust source is not syntactically valid"));
        }
        legacy
    };
    let ast = parsed.tree();
    context.check()?;
    let _timer = index_profile::operation("rust_ast_analysis");
    consume(&ast, base)
}
