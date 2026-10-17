//! All AST/spans stay on the current thread and die before its source arena is reset.
use harness_domain::{Failure, execution::OperationContext};

struct ResetSpans;
impl Drop for ResetSpans {
    fn drop(&mut self) {
        proc_macro2::extra::invalidate_current_thread_spans();
    }
}

/// Callers may return owned data, never AST nodes, tokens or spans.
pub(super) fn with_ast<T>(
    source: &str,
    context: &OperationContext,
    consume: impl FnOnce(&syn::File, usize) -> Result<T, Failure>,
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
    let _reset = ResetSpans;
    let ast = syn::parse_file(&prepared)
        .map_err(|_| ("SYNTAX_ERROR", "Rust source is not syntactically valid"))?;
    context.check()?;
    consume(&ast, base)
}
