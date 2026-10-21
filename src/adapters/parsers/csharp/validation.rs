use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};

use super::lexer::{MAX_CSHARP_SIZE, TokenKind, line_number, tokenize};

pub(in crate::adapters) fn validate_syntax(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cshtml"))
    {
        return crate::adapters::parsers::razor::syntax::validate(path, source, context);
    }
    if source.len() > MAX_CSHARP_SIZE {
        return Err(("FILE_TOO_LARGE", "C# file exceeds max allowed size"));
    }

    // Tokenize to confirm strings/comments and brackets are well-formed
    let tokens = tokenize(source, context)?;
    let mut depth = 0;
    for token in &tokens {
        match token.kind {
            TokenKind::OpenBrace => depth += 1,
            TokenKind::CloseBrace => {
                if depth == 0 {
                    let _line = line_number(source, token.start);
                    return Err(("SYNTAX_ERROR", "Unmatched closing brace '}'"));
                }
                depth -= 1;
            }
            _ => {}
        }
    }

    if depth != 0 && !path.ends_with(".cshtml") {
        return Err(("SYNTAX_ERROR", "Unclosed opening brace '{'"));
    }

    Ok(json!({
        "status": "checked",
        "language": if path.ends_with(".cshtml") { "razor" } else { "csharp" },
        "errors": 0,
        "diagnostics": []
    }))
}
