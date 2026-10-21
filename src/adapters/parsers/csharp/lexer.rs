use harness_domain::{Failure, execution::OperationContext};

pub(super) const MAX_CSHARP_SIZE: usize = 4 * 1024 * 1024;

pub(super) fn line_number(source: &str, byte_offset: usize) -> usize {
    source.as_bytes()[..byte_offset.min(source.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

/// Counts the consecutive quote bytes at `start`.
fn quote_run(bytes: &[u8], start: usize, len: usize) -> usize {
    let mut run = 0;
    while start + run < len && bytes[start + run] == b'"' {
        run += 1;
    }
    run
}

/// Skips a C# 11 raw string literal that opens with `quotes` quote bytes.
///
/// The closing delimiter is a run of the same number of quotes. A longer run
/// ends the literal as well, because the extra quotes belong to the content.
fn raw_string_end(bytes: &[u8], start: usize, quotes: usize, len: usize) -> usize {
    let mut i = start + quotes;
    while i < len {
        if bytes[i] == b'"' {
            let run = quote_run(bytes, i, len);
            if run >= quotes {
                return i + run;
            }
            i += run;
            continue;
        }
        i += 1;
    }
    len
}

/// Tokenizer/Lexer state for C# that strips comments and strings while tracking byte ranges.
#[derive(Debug, Clone)]
pub(super) enum TokenKind {
    Word(String),
    OpenBrace,
    CloseBrace,
    OpenParen,
    CloseParen,
    Semicolon,
    Colon,
    OpenBracket,
    CloseBracket,
    DocComment(String),
}

#[derive(Debug, Clone)]
pub(super) struct Token {
    pub(super) kind: TokenKind,
    pub(super) start: usize,
    pub(super) end: usize,
}

/// Lexes C# source, capturing words, punctuation, and XML doc comments while safely skipping strings and comments.
pub(super) fn tokenize(source: &str, context: &OperationContext) -> Result<Vec<Token>, Failure> {
    context.check()?;
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < len {
        if i % 8192 == 0 {
            context.check()?;
        }
        let b = bytes[i];

        // Skip whitespace
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Line comments or Doc comments
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            let is_doc =
                i + 2 < len && bytes[i + 2] == b'/' && (i + 3 >= len || bytes[i + 3] != b'/');
            let start = i;
            i += 2;
            let doc_start = i + if is_doc { 1 } else { 0 };
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            if is_doc {
                let doc_text = source[doc_start..i].trim().to_string();
                tokens.push(Token {
                    kind: TokenKind::DocComment(doc_text),
                    start,
                    end: i,
                });
            }
            continue;
        }

        // Block comments /* ... */
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < len {
                i += 2;
            } else {
                i = len;
            }
            continue;
        }

        // Raw string literals (C# 11): """...""" and $"""..."""
        if b == b'"' {
            let run = quote_run(bytes, i, len);
            if run >= 3 {
                i = raw_string_end(bytes, i, run, len);
                continue;
            }
        }

        // Verbatim or raw strings: @"..." or $@"..." or @$"..."
        if (b == b'@' || b == b'$') && i + 1 < len {
            let mut prefix_len = 1;
            let mut is_verbatim = b == b'@';
            if (bytes[i + 1] == b'@' || bytes[i + 1] == b'$') && bytes[i] != bytes[i + 1] {
                prefix_len = 2;
                is_verbatim = true;
            }
            if i + prefix_len < len && bytes[i + prefix_len] == b'"' {
                let run = quote_run(bytes, i + prefix_len, len);
                if run >= 3 {
                    // Interpolated raw string: $"""..."""
                    i = raw_string_end(bytes, i + prefix_len, run, len);
                    continue;
                }
                i += prefix_len + 1;
                if is_verbatim {
                    while i < len {
                        if bytes[i] == b'"' {
                            if i + 1 < len && bytes[i + 1] == b'"' {
                                i += 2; // escaped quote ""
                            } else {
                                i += 1; // closing quote
                                break;
                            }
                        } else {
                            i += 1;
                        }
                    }
                } else {
                    // regular interpolated string
                    while i < len {
                        if bytes[i] == b'\\' && i + 1 < len {
                            i += 2;
                        } else if bytes[i] == b'"' {
                            i += 1;
                            break;
                        } else {
                            i += 1;
                        }
                    }
                }
                continue;
            }
        }

        // Standard string "..."
        if b == b'"' {
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }

        // Character literal '...'
        if b == b'\'' {
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == b'\'' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }

        // Single character punctuations
        match b {
            b'{' => {
                tokens.push(Token {
                    kind: TokenKind::OpenBrace,
                    start: i,
                    end: i + 1,
                });
                i += 1;
                continue;
            }
            b'}' => {
                tokens.push(Token {
                    kind: TokenKind::CloseBrace,
                    start: i,
                    end: i + 1,
                });
                i += 1;
                continue;
            }
            b'(' => {
                tokens.push(Token {
                    kind: TokenKind::OpenParen,
                    start: i,
                    end: i + 1,
                });
                i += 1;
                continue;
            }
            b')' => {
                tokens.push(Token {
                    kind: TokenKind::CloseParen,
                    start: i,
                    end: i + 1,
                });
                i += 1;
                continue;
            }
            b';' => {
                tokens.push(Token {
                    kind: TokenKind::Semicolon,
                    start: i,
                    end: i + 1,
                });
                i += 1;
                continue;
            }
            b':' => {
                tokens.push(Token {
                    kind: TokenKind::Colon,
                    start: i,
                    end: i + 1,
                });
                i += 1;
                continue;
            }
            b'[' => {
                tokens.push(Token {
                    kind: TokenKind::OpenBracket,
                    start: i,
                    end: i + 1,
                });
                i += 1;
                continue;
            }
            b']' => {
                tokens.push(Token {
                    kind: TokenKind::CloseBracket,
                    start: i,
                    end: i + 1,
                });
                i += 1;
                continue;
            }
            _ => {}
        }

        // Word (identifier or keyword)
        if b.is_ascii_alphanumeric() || b == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = source[start..i].to_string();
            tokens.push(Token {
                kind: TokenKind::Word(word),
                start,
                end: i,
            });
            continue;
        }

        i += 1;
    }

    Ok(tokens)
}

/// Matches braces { } to determine container spans and body ends.
pub(super) fn find_matching_brace(tokens: &[Token], open_brace_index: usize) -> Option<usize> {
    let mut depth = 0;
    for (idx, token) in tokens.iter().enumerate().skip(open_brace_index) {
        match token.kind {
            TokenKind::OpenBrace => depth += 1,
            TokenKind::CloseBrace => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}
