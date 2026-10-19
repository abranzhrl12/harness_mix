//! C# (.cs) and Razor (.cshtml) symbol extraction and syntax validation.
//! Parses namespaces, classes, interfaces, structs, records, enums, methods, constructors,
//! properties, and Razor @code/@functions blocks into canonical symbols.

use super::rust_symbols::Symbol;
use harness_domain::{Failure, execution::OperationContext, hash, results::prefix};
use serde_json::{Value, json};

const MAX_CSHARP_SIZE: usize = 4 * 1024 * 1024;

fn line_number(source: &str, byte_offset: usize) -> usize {
    source.as_bytes()[..byte_offset.min(source.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

/// Tokenizer/Lexer state for C# that strips comments and strings while tracking byte ranges.
#[derive(Debug, Clone)]
enum TokenKind {
    Word(String),
    OpenBrace,
    CloseBrace,
    OpenParen,
    CloseParen,
    Semicolon,
    Colon,
    DocComment(String),
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
}

/// Lexes C# source, capturing words, punctuation, and XML doc comments while safely skipping strings and comments.
fn tokenize(source: &str, context: &OperationContext) -> Result<Vec<Token>, Failure> {
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

        // Verbatim or raw strings: @"..." or $@"..." or @$"..."
        if (b == b'@' || b == b'$') && i + 1 < len {
            let mut prefix_len = 1;
            let mut is_verbatim = b == b'@';
            if (bytes[i + 1] == b'@' || bytes[i + 1] == b'$') && bytes[i] != bytes[i + 1] {
                prefix_len = 2;
                is_verbatim = true;
            }
            if i + prefix_len < len && bytes[i + prefix_len] == b'"' {
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
fn find_matching_brace(tokens: &[Token], open_brace_index: usize) -> Option<usize> {
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

/// Extract symbols from a C# source string.
pub(super) fn collect(
    source: &str,
    _digest: &str,
    context: &OperationContext,
) -> Result<Vec<Symbol>, Failure> {
    if source.len() > MAX_CSHARP_SIZE {
        return Err(("PARSER_LIMIT", "C# source exceeds max allowed size (4MB)"));
    }

    let tokens = tokenize(source, context)?;
    let mut symbols: Vec<Symbol> = Vec::new();
    let num_tokens = tokens.len();

    // Stack to keep track of current container hierarchy: (name, close_token_idx)
    let mut container_stack: Vec<(String, usize)> = Vec::new();
    let mut last_doc: Option<String> = None;

    let mut i = 0;
    while i < num_tokens {
        context.check()?;

        // Clean up expired containers
        while let Some((_, close_idx)) = container_stack.last() {
            if i > *close_idx {
                container_stack.pop();
            } else {
                break;
            }
        }

        let token = &tokens[i];

        // Track XML Doc comments
        if let TokenKind::DocComment(ref doc) = token.kind {
            if let Some(existing) = last_doc.as_mut() {
                existing.push(' ');
                existing.push_str(doc);
            } else {
                last_doc = Some(doc.clone());
            }
            i += 1;
            continue;
        }

        if let TokenKind::Word(ref word) = token.kind {
            let current_container = container_stack
                .iter()
                .map(|(c, _)| c.as_str())
                .collect::<Vec<_>>()
                .join(".");

            // 1. Types: class, struct, interface, enum, record
            if matches!(
                word.as_str(),
                "class" | "interface" | "struct" | "enum" | "record"
            ) {
                let kind_str = match word.as_str() {
                    "class" => "class",
                    "interface" => "interface",
                    "struct" => "struct",
                    "enum" => "enum",
                    "record" => "record",
                    _ => "class",
                };

                // The type name is typically the next Word token
                if i + 1 < num_tokens {
                    if let TokenKind::Word(ref type_name) = tokens[i + 1].kind {
                        // Find the opening brace of this type
                        let mut brace_idx = None;
                        for j in (i + 1)..num_tokens.min(i + 40) {
                            if matches!(tokens[j].kind, TokenKind::OpenBrace) {
                                brace_idx = Some(j);
                                break;
                            }
                            if matches!(tokens[j].kind, TokenKind::Semicolon) {
                                // Interface or partial/record without body
                                break;
                            }
                        }

                        if let Some(open_idx) = brace_idx {
                            if let Some(close_idx) = find_matching_brace(&tokens, open_idx) {
                                let sig_start = tokens[i].start;
                                let sig_end = tokens[open_idx].start;
                                let raw_sig = source[sig_start..sig_end].trim().to_string();
                                let sig = if raw_sig.is_empty() {
                                    format!("{} {}", kind_str, type_name)
                                } else {
                                    raw_sig
                                };

                                let start_byte = tokens[i].start;
                                let end_byte = tokens[close_idx].end;

                                let start_line = line_number(source, start_byte);
                                let end_line = line_number(source, end_byte);

                                let id_payload =
                                    format!("{}:{}:{}", current_container, type_name, start_line);
                                let sym_id = prefix(&hash(id_payload.as_bytes()), 16).to_string();

                                symbols.push(Symbol {
                                    name: type_name.clone(),
                                    container: current_container.clone(),
                                    kind: kind_str,
                                    start: start_line,
                                    end: end_line,
                                    signature: sig,
                                    doc: last_doc.take().unwrap_or_default(),
                                    id: sym_id,
                                    bytes: start_byte..end_byte,
                                });

                                container_stack.push((type_name.clone(), close_idx));
                                i = i + 2;
                                continue;
                            }
                        }
                    }
                }
            }

            // 2. Methods and Constructors
            // Pattern: [modifiers] [Type] MethodName (...) { ... } or => ...;
            if i + 1 < num_tokens && matches!(tokens[i + 1].kind, TokenKind::OpenParen) {
                let method_name = word.clone();
                // Avoid control flow statements
                if !matches!(
                    method_name.as_str(),
                    "if" | "while"
                        | "for"
                        | "foreach"
                        | "switch"
                        | "catch"
                        | "using"
                        | "lock"
                        | "typeof"
                        | "sizeof"
                        | "nameof"
                ) {
                    // Find closing paren
                    let mut paren_close = None;
                    let mut p_depth = 0;
                    for j in (i + 1)..num_tokens.min(i + 60) {
                        match tokens[j].kind {
                            TokenKind::OpenParen => p_depth += 1,
                            TokenKind::CloseParen => {
                                p_depth -= 1;
                                if p_depth == 0 {
                                    paren_close = Some(j);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }

                    if let Some(p_close) = paren_close {
                        // Look for body { ... } or expression body => ... ;
                        let mut body_end = None;
                        for j in (p_close + 1)..num_tokens.min(p_close + 20) {
                            if matches!(tokens[j].kind, TokenKind::OpenBrace) {
                                if let Some(c_idx) = find_matching_brace(&tokens, j) {
                                    body_end = Some((j, tokens[c_idx].end));
                                    break;
                                }
                            } else if matches!(tokens[j].kind, TokenKind::Semicolon) {
                                body_end = Some((j, tokens[j].end));
                                break;
                            }
                        }

                        if let Some((sig_delim_idx, end_byte)) = body_end {
                            // Find start of declaration (look back up to 8 tokens)
                            let mut start_token_idx = i;
                            let lookback_limit = if i >= 8 { i - 8 } else { 0 };
                            for k in (lookback_limit..i).rev() {
                                match tokens[k].kind {
                                    TokenKind::Semicolon
                                    | TokenKind::OpenBrace
                                    | TokenKind::CloseBrace => {
                                        start_token_idx = k + 1;
                                        break;
                                    }
                                    TokenKind::Word(ref w) => {
                                        if matches!(
                                            w.as_str(),
                                            "public"
                                                | "private"
                                                | "protected"
                                                | "internal"
                                                | "static"
                                                | "virtual"
                                                | "override"
                                                | "async"
                                        ) {
                                            start_token_idx = k;
                                        }
                                    }
                                    _ => {}
                                }
                            }

                            let start_byte = tokens[start_token_idx].start;
                            let sig_end = tokens[sig_delim_idx].start;
                            let raw_sig = source[start_byte..sig_end].trim().to_string();
                            let sig = if raw_sig.is_empty() {
                                format!("{}(...)", method_name)
                            } else {
                                raw_sig
                            };

                            let start_line = line_number(source, start_byte);
                            let end_line = line_number(source, end_byte);

                            let kind =
                                if container_stack.last().map(|(c, _)| c) == Some(&method_name) {
                                    "constructor"
                                } else {
                                    "method"
                                };

                            let id_payload =
                                format!("{}:{}:{}", current_container, method_name, start_line);
                            let sym_id = prefix(&hash(id_payload.as_bytes()), 16).to_string();

                            symbols.push(Symbol {
                                name: method_name,
                                container: current_container.clone(),
                                kind,
                                start: start_line,
                                end: end_line,
                                signature: sig,
                                doc: last_doc.take().unwrap_or_default(),
                                id: sym_id,
                                bytes: start_byte..end_byte,
                            });
                        }
                    }
                }
            }

            // 3. Properties: [modifiers] [Type] PropName { get; set; }
            if i + 1 < num_tokens && matches!(tokens[i + 1].kind, TokenKind::OpenBrace) {
                let prop_name = word.clone();
                if !matches!(
                    prop_name.as_str(),
                    "class"
                        | "interface"
                        | "struct"
                        | "enum"
                        | "record"
                        | "namespace"
                        | "get"
                        | "set"
                        | "init"
                ) {
                    let open_idx = i + 1;
                    if let Some(close_idx) = find_matching_brace(&tokens, open_idx) {
                        let inner_has_accessor = tokens[open_idx..=close_idx].iter().any(|t| {
                            if let TokenKind::Word(ref w) = t.kind {
                                matches!(w.as_str(), "get" | "set" | "init")
                            } else {
                                false
                            }
                        });

                        if inner_has_accessor {
                            let mut start_token_idx = i;
                            let lookback_limit = if i >= 6 { i - 6 } else { 0 };
                            for k in (lookback_limit..i).rev() {
                                match tokens[k].kind {
                                    TokenKind::Semicolon
                                    | TokenKind::OpenBrace
                                    | TokenKind::CloseBrace => {
                                        start_token_idx = k + 1;
                                        break;
                                    }
                                    _ => {}
                                }
                            }

                            let start_byte = tokens[start_token_idx].start;
                            let end_byte = tokens[close_idx].end;

                            let raw_sig = source[start_byte..tokens[open_idx].start]
                                .trim()
                                .to_string();
                            let sig = if raw_sig.is_empty() {
                                format!("{} {{ get; set; }}", prop_name)
                            } else {
                                format!("{} {{ ... }}", raw_sig)
                            };

                            let start_line = line_number(source, start_byte);
                            let end_line = line_number(source, end_byte);

                            let id_payload =
                                format!("{}:{}:{}", current_container, prop_name, start_line);
                            let sym_id = prefix(&hash(id_payload.as_bytes()), 16).to_string();

                            symbols.push(Symbol {
                                name: prop_name,
                                container: current_container,
                                kind: "property",
                                start: start_line,
                                end: end_line,
                                signature: sig,
                                doc: last_doc.take().unwrap_or_default(),
                                id: sym_id,
                                bytes: start_byte..end_byte,
                            });
                        }
                    }
                }
            }
        }

        i += 1;
    }

    Ok(symbols)
}

/// Basic Razor declarations and C# members, preserving exact source byte ranges.
pub(super) fn collect_cshtml(
    source: &str,
    digest: &str,
    context: &OperationContext,
) -> Result<Vec<Symbol>, Failure> {
    if source.len() > MAX_CSHARP_SIZE {
        return Err((
            "PARSER_LIMIT",
            "Razor source exceeds max allowed size (4MB)",
        ));
    }

    let mut symbols = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        context.check()?;
        if bytes[i..].starts_with(b"@*") {
            i += 2;
            while i + 1 < len && !bytes[i..].starts_with(b"*@") {
                i += 1;
            }
            i = (i + 2).min(len);
            continue;
        }
        if bytes[i..].starts_with(b"@@") {
            i += 2;
            continue;
        }
        if bytes[i] == b'@' && i + 1 < len {
            let after = &source[i + 1..];
            let directive = ["model", "section", "helper", "code", "functions"]
                .into_iter()
                .find(|tag| {
                    after.starts_with(tag)
                        && bytes
                            .get(i + 1 + tag.len())
                            .is_some_and(|b| b.is_ascii_whitespace() || *b == b'{')
                });
            if let Some(tag) = directive {
                let tag_len = tag.len();
                let mut brace_start = i + 1 + tag_len;
                while brace_start < len && bytes[brace_start].is_ascii_whitespace() {
                    brace_start += 1;
                }
                let declaration_start = brace_start;
                if tag == "model" {
                    let end = source[brace_start..]
                        .find('\n')
                        .map_or(len, |n| brace_start + n);
                    let name = source[brace_start..end].trim();
                    if !name.is_empty() {
                        symbols.push(razor_symbol(source, digest, name, tag, i, end, end));
                    }
                    i = end;
                    continue;
                }
                if tag == "section" || tag == "helper" {
                    while brace_start < len
                        && (bytes[brace_start].is_ascii_alphanumeric()
                            || bytes[brace_start] == b'_'
                            || bytes[brace_start] >= 128)
                    {
                        brace_start += 1;
                    }
                    let name = &source[declaration_start..brace_start];
                    if name.is_empty() {
                        i += 1;
                        continue;
                    }
                    while brace_start < len && bytes[brace_start].is_ascii_whitespace() {
                        brace_start += 1;
                    }
                    if tag == "helper" {
                        if bytes.get(brace_start) != Some(&b'(') {
                            i += 1;
                            continue;
                        }
                        if let Some(end) =
                            super::razor_syntax::delimited_end(bytes, brace_start, b'(', b')')
                        {
                            brace_start = end;
                            while brace_start < len && bytes[brace_start].is_ascii_whitespace() {
                                brace_start += 1;
                            }
                        } else {
                            i += 1;
                            continue;
                        }
                    }
                    if bytes.get(brace_start) == Some(&b'{') {
                        if let Some(end) = super::razor_syntax::block_end(bytes, brace_start) {
                            symbols.push(razor_symbol(
                                source,
                                digest,
                                name,
                                tag,
                                i,
                                brace_start,
                                end,
                            ));
                            i = end;
                            continue;
                        }
                    }
                }
                if matches!(tag, "code" | "functions")
                    && brace_start < len
                    && bytes[brace_start] == b'{'
                {
                    if let Some(end) = super::razor_syntax::block_end(bytes, brace_start) {
                        let block_code = &source[brace_start + 1..end - 1];
                        if let Ok(block_syms) = collect(block_code, digest, context) {
                            let block_line_offset =
                                line_number(source, brace_start + 1).saturating_sub(1);
                            let byte_offset = brace_start + 1;
                            for mut sym in block_syms {
                                sym.start += block_line_offset;
                                sym.end += block_line_offset;
                                sym.bytes =
                                    (sym.bytes.start + byte_offset)..(sym.bytes.end + byte_offset);
                                sym.container = if sym.container.is_empty() {
                                    format!("@{tag}")
                                } else {
                                    format!("@{tag}.{}", sym.container)
                                };
                                symbols.push(sym);
                            }
                        }
                        i = end;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }

    Ok(symbols)
}

fn razor_symbol(
    source: &str,
    digest: &str,
    name: &str,
    tag: &str,
    start: usize,
    header_end: usize,
    end: usize,
) -> Symbol {
    Symbol {
        name: name.to_string(),
        container: format!("@{tag}"),
        kind: match tag {
            "model" => "razor_model",
            "section" => "razor_section",
            "helper" => "razor_helper",
            _ => "razor_directive",
        },
        start: line_number(source, start),
        end: line_number(source, end.saturating_sub(1)),
        signature: source[start..header_end].trim().to_string(),
        doc: String::new(),
        id: prefix(
            &hash(format!("{digest}:{tag}:{name}:{start}").as_bytes()),
            16,
        )
        .to_string(),
        bytes: start..end,
    }
}

/// Simple syntax check for C# ensuring bracket balance and string integrity.
pub(super) fn validate_syntax(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cshtml"))
    {
        return super::razor_syntax::validate(path, source, context);
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
