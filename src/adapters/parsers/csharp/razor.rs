use crate::adapters::parsers::model::Symbol;
use harness_domain::{Failure, execution::OperationContext, hash, results::prefix};

use super::declarations::symbols::collect;
use super::lexer::{MAX_CSHARP_SIZE, line_number};

pub(in crate::adapters) fn razor_markup_end(bytes: &[u8], start: usize) -> Option<usize> {
    let tail = &bytes[start..];
    if tail.starts_with(b"<!--") {
        return Some(
            tail.windows(3)
                .position(|w| w == b"-->")
                .map_or(bytes.len(), |n| start + n + 3),
        );
    }
    if tail.first() != Some(&b'<')
        || !tail
            .get(1)
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'/')
    {
        return None;
    }
    let mut i = start + 1;
    let mut quote = None;
    while i < bytes.len() {
        match (quote, bytes[i]) {
            (Some(q), b) if q == b => quote = None,
            (None, b'\'' | b'"') => quote = Some(bytes[i]),
            (None, b'>') => break,
            _ => {}
        }
        i += 1;
    }
    let end = (i + 1).min(bytes.len());
    for tag in [b"script".as_slice(), b"style".as_slice()] {
        if tail
            .get(1..1 + tag.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(tag))
            && tail
                .get(1 + tag.len())
                .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>')
        {
            let closing = [b"</".as_slice(), tag].concat();
            if let Some(n) = bytes[end..]
                .windows(closing.len())
                .position(|s| s.eq_ignore_ascii_case(&closing))
            {
                let close = end + n + closing.len();
                return Some(
                    bytes[close..]
                        .iter()
                        .position(|b| *b == b'>')
                        .map_or(bytes.len(), |n| close + n + 1),
                );
            }
            return Some(bytes.len());
        }
    }
    Some(end)
}

pub(in crate::adapters) fn collect_cshtml(
    source: &str,
    digest: &str,
    context: &OperationContext,
) -> Result<Vec<Symbol>, Failure> {
    collect_cshtml_blocks(source, digest, context, &mut Vec::new(), true)
}

/// Shared Razor scan: expose C# block bodies for native graph extraction without
/// rescanning directives or losing source positions. Ranges exclude Razor braces.
pub(in crate::adapters) fn collect_cshtml_blocks(
    source: &str,
    digest: &str,
    context: &OperationContext,
    blocks: &mut Vec<(std::ops::Range<usize>, &'static str)>,
    include_members: bool,
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
        if let Some(end) = razor_markup_end(bytes, i) {
            i = end;
            continue;
        }
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
                        if let Some(end) = crate::adapters::parsers::razor::syntax::delimited_end(
                            bytes,
                            brace_start,
                            b'(',
                            b')',
                        ) {
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
                        if let Some(end) =
                            crate::adapters::parsers::razor::syntax::block_end(bytes, brace_start)
                        {
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
                    if let Some(end) =
                        crate::adapters::parsers::razor::syntax::block_end(bytes, brace_start)
                    {
                        blocks.push((brace_start + 1..end - 1, tag));
                        if !include_members {
                            i = end;
                            continue;
                        }
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
