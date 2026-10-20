//! Conservative CSS rule spans, preserving the original file offsets for edits.
use super::Symbol;
use harness_domain::{Failure, execution::OperationContext, hash};

pub(super) fn collect(
    source: &str,
    digest: &str,
    embedded: bool,
    context: &OperationContext,
) -> Result<Vec<Symbol>, Failure> {
    let max_bytes = if embedded {
        4 * 1024 * 1024
    } else {
        256 * 1024
    };
    if source.len() > max_bytes {
        return Err(("PARSER_LIMIT", "CSS source budget exceeded"));
    }
    let mut result = Vec::new();
    if embedded {
        let lower = source.to_ascii_lowercase();
        let mut cursor = 0;
        while let Some(relative) = lower[cursor..].find('<') {
            context.check()?;
            let tag = cursor + relative;
            if lower[tag..].starts_with("<!--") {
                cursor = lower[tag + 4..]
                    .find("-->")
                    .map_or(lower.len(), |end| tag + 4 + end + 3);
                continue;
            }
            let mut quote = 0;
            cursor = tag + 1;
            while cursor < lower.len() {
                let b = lower.as_bytes()[cursor];
                if quote != 0 {
                    if b == quote {
                        quote = 0;
                    }
                } else if b == b'\'' || b == b'"' {
                    quote = b;
                } else if b == b'>' {
                    break;
                }
                cursor += 1;
            }
            if cursor == lower.len() {
                break;
            }
            cursor += 1;
            for name in ["style", "script"] {
                let name_end = tag + 1 + name.len();
                if lower[tag + 1..].starts_with(name)
                    && lower
                        .as_bytes()
                        .get(name_end)
                        .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>')
                {
                    let closing = format!("</{name}");
                    let Some(close) = lower[cursor..].find(&closing) else {
                        break;
                    };
                    let end = cursor + close;
                    if name == "style" {
                        scan(source, digest, cursor, end, "", context, &mut result)?;
                    }
                    cursor = end + closing.len();
                    break;
                }
            }
        }
    } else {
        scan(source, digest, 0, source.len(), "", context, &mut result)?;
    }
    result.sort_by_key(|item| item.bytes.start);
    Ok(result)
}

fn scan(
    source: &str,
    digest: &str,
    start: usize,
    end: usize,
    container: &str,
    context: &OperationContext,
    result: &mut Vec<Symbol>,
) -> Result<(), Failure> {
    let bytes = source.as_bytes();
    let mut i = start;
    let mut head = start;
    let mut depth = 0usize;
    let mut open = 0;
    let mut quote = 0;
    let mut parens = 0usize;
    let mut brackets = 0usize;
    while i < end {
        context.check()?;
        let b = bytes[i];
        if quote != 0 {
            if b == b'\\' {
                i = (i + 2).min(end);
                continue;
            }
            if b == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let Some(close) = source[i + 2..end].find("*/") else {
                break;
            };
            let next = i + close + 4;
            if depth == 0 && source[head..i].trim().is_empty() {
                head = next;
            }
            i = next;
            continue;
        }
        if b == b'\\' {
            i = (i + 2).min(end);
            continue;
        }
        if b == b'\'' || b == b'"' {
            quote = b;
        } else if b == b'(' {
            parens += 1;
        } else if b == b')' {
            parens = parens.saturating_sub(1);
        } else if b == b'[' {
            brackets += 1;
        } else if b == b']' {
            brackets = brackets.saturating_sub(1);
        } else if parens == 0 && brackets == 0 {
            match b {
                b'{' => {
                    if depth == 0 {
                        open = i;
                    }
                    depth += 1;
                }
                b'}' if depth > 0 => {
                    depth -= 1;
                    if depth == 0 {
                        let selector = source[head..open].trim();
                        if !selector.is_empty() {
                            if result.len() >= 1000
                                || selector.len() > 1024
                                || container.len() > 4096
                            {
                                return Err(("PARSER_LIMIT", "CSS symbol budget exceeded"));
                            }
                            let from = head + source[head..open].len()
                                - source[head..open].trim_start().len();
                            let to = i + 1;
                            let at_rule = selector.starts_with('@');
                            result.push(Symbol {
                                name: selector.into(),
                                container: container.into(),
                                kind: if at_rule { "css-at-rule" } else { "css-rule" },
                                start: source[..from].bytes().filter(|b| *b == b'\n').count() + 1,
                                end: source[..to].bytes().filter(|b| *b == b'\n').count() + 1,
                                signature: format!("{selector} {{ … }}"),
                                doc: String::new(),
                                id: hash(format!("{digest}:{from}:{to}:css").as_bytes())[..24]
                                    .into(),
                                bytes: from..to,
                            });
                            // Grouping at-rules contain rule lists; declaration at-rules do not.
                            let keyword = selector.split_whitespace().next().unwrap_or("");
                            if matches!(
                                keyword,
                                "@media"
                                    | "@supports"
                                    | "@layer"
                                    | "@container"
                                    | "@scope"
                                    | "@starting-style"
                                    | "@keyframes"
                                    | "@-webkit-keyframes"
                            ) {
                                if container.matches(" / ").count() >= 64 {
                                    return Err(("PARSER_LIMIT", "CSS nesting budget exceeded"));
                                }
                                let nested = if container.is_empty() {
                                    selector.to_owned()
                                } else {
                                    format!("{container} / {selector}")
                                };
                                scan(source, digest, open + 1, i, &nested, context, result)?;
                            }
                        }
                        head = i + 1;
                    }
                }
                b';' if depth == 0 => head = i + 1,
                _ => {}
            }
        }
        i += 1;
    }
    Ok(())
}
