use crate::adapters::parsers::model::Symbol;
use harness_domain::{Failure, execution::OperationContext, hash, results::prefix};

use super::super::lexer::{MAX_CSHARP_SIZE, TokenKind, find_matching_brace, line_number, tokenize};

pub(in crate::adapters) fn collect(
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
    let mut skip_method_until: Option<usize> = None;

    let mut i = 0;
    while i < num_tokens {
        context.check()?;

        if let Some(end_byte) = skip_method_until {
            if tokens[i].start < end_byte {
                i += 1;
                continue;
            }
            skip_method_until = None;
        }

        // Clean up expired containers
        while let Some((_, close_idx)) = container_stack.last() {
            if i > *close_idx {
                container_stack.pop();
            } else {
                break;
            }
        }

        let token = &tokens[i];

        // Attribute names can look exactly like methods (for example,
        // [Route("...")]). Do not run declaration discovery over the contents
        // of a bracketed attribute/index expression; declaration-prefix
        // extraction below still reads attributes from the original tokens.
        if matches!(token.kind, TokenKind::OpenBracket) {
            let mut depth = 1usize;
            let mut close = None;
            for (index, candidate) in tokens.iter().enumerate().skip(i + 1) {
                context.check()?;
                match candidate.kind {
                    TokenKind::OpenBracket => depth += 1,
                    TokenKind::CloseBracket => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(index);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(close) = close {
                i = close + 1;
                continue;
            }
            return Err((
                "SYNTAX_ERROR",
                "Unterminated C# attribute or index expression",
            ));
        }

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
                                    attributes: Vec::new(),
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
                            // Find the declaration prefix back to the previous structural
                            // boundary. Attributes can span more than eight tokens (including
                            // multiple and multiline attributes), so a fixed lookback can
                            // silently cut the prefix off.
                            let mut start_token_idx = None;
                            for k in (0..i).rev() {
                                match tokens[k].kind {
                                    TokenKind::Semicolon
                                    | TokenKind::OpenBrace
                                    | TokenKind::CloseBrace => {
                                        if start_token_idx.is_none() {
                                            start_token_idx = Some(k + 1);
                                        }
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
                                            start_token_idx = Some(k);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            let start_token_idx = start_token_idx.unwrap_or(i);

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
                                attributes: attributes_before(&tokens, start_token_idx, source),
                                doc: last_doc.take().unwrap_or_default(),
                                id: sym_id,
                                bytes: start_byte..end_byte,
                            });
                            skip_method_until = Some(end_byte);
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
                                attributes: Vec::new(),
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

fn attributes_before(
    tokens: &[super::super::lexer::Token],
    declaration: usize,
    source: &str,
) -> Vec<String> {
    use super::super::lexer::TokenKind;

    let mut cursor = declaration;
    let mut attributes = Vec::new();
    loop {
        let Some(close) = cursor.checked_sub(1) else {
            break;
        };
        if !matches!(tokens[close].kind, TokenKind::CloseBracket) {
            break;
        }
        let mut depth = 0usize;
        let mut open = None;
        for index in (0..=close).rev() {
            match tokens[index].kind {
                TokenKind::CloseBracket => depth += 1,
                TokenKind::OpenBracket => {
                    depth -= 1;
                    if depth == 0 {
                        open = Some(index);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(open) = open else {
            break;
        };
        attributes.push(
            source[tokens[open].start..tokens[close].end]
                .trim()
                .to_owned(),
        );
        cursor = open;
    }
    attributes.reverse();
    attributes
}
