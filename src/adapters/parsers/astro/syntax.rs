//! Astro component syntax validation and symbol extraction.
//! Validates frontmatter TypeScript/JavaScript and inline template scripts using Oxc AST.
use crate::adapters::parsers::{
    ecmascript::{symbols as ts_symbols, syntax as ts_syntax},
    model::Symbol,
    vue::syntax as lex,
};
use harness_domain::{Failure, execution::OperationContext, hash, results::prefix};
use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use oxc_parser::Parser;
use oxc_span::SourceType;
use serde_json::{Value, json};

const MAX_ASTRO: usize = 2 * 1024 * 1024;

fn line(source: &str, at: usize) -> usize {
    source.as_bytes()[..at.min(source.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

/// Finds the frontmatter fences `--- ... ---` in an Astro file.
/// Returns `Ok(Some((body_start, body_end, after_fence)))` if frontmatter is present.
/// Returns `Ok(None)` if file does not begin with frontmatter.
/// Returns `Err(SYNTAX_ERROR)` if frontmatter is opened with `---` but never closed.
pub(in crate::adapters) fn find_frontmatter(
    source: &str,
) -> Result<Option<(usize, usize, usize)>, Failure> {
    let clean = source.strip_prefix('\u{feff}').unwrap_or(source);
    let offset = source.len() - clean.len();
    let trimmed = clean.trim_start();
    if !trimmed.starts_with("---") {
        return Ok(None);
    }
    // Opening fence must be at start of source (ignoring whitespace/BOM)
    let fence_start = offset + (clean.len() - trimmed.len());
    let after_open = fence_start + 3;
    let bytes = source.as_bytes();

    // Opening fence must be followed by newline or whitespace until newline
    let mut open_nl = after_open;
    while open_nl < bytes.len() && bytes[open_nl] != b'\n' {
        if !bytes[open_nl].is_ascii_whitespace() {
            // Not a frontmatter fence (e.g. `---something`)
            return Ok(None);
        }
        open_nl += 1;
    }
    if open_nl < bytes.len() && bytes[open_nl] == b'\n' {
        open_nl += 1;
    }
    let body_start = open_nl;

    // Search for closing `---` at the beginning of a line
    let mut cursor = body_start;
    while cursor < bytes.len() {
        // Line start
        let mut line_cursor = cursor;
        while line_cursor < bytes.len()
            && (bytes[line_cursor] == b' ' || bytes[line_cursor] == b'\t')
        {
            line_cursor += 1;
        }
        if source[line_cursor..].starts_with("---") {
            let after_close = line_cursor + 3;
            // Closing fence must be followed by newline, EOF, or whitespace until newline
            let mut end_line = after_close;
            let mut valid_close = true;
            while end_line < bytes.len() && bytes[end_line] != b'\n' {
                if !bytes[end_line].is_ascii_whitespace() && bytes[end_line] != b'\r' {
                    valid_close = false;
                    break;
                }
                end_line += 1;
            }
            if valid_close {
                let body_end = cursor;
                let after_fence = if end_line < bytes.len() && bytes[end_line] == b'\n' {
                    end_line + 1
                } else {
                    end_line
                };
                return Ok(Some((body_start, body_end, after_fence)));
            }
        }
        // Move to next line
        if let Some(nl) = source[cursor..].find('\n') {
            cursor += nl + 1;
        } else {
            break;
        }
    }
    Err((
        "SYNTAX_ERROR",
        "Unterminated Astro frontmatter fence (missing closing ---)",
    ))
}

/// Validate Astro syntax: frontmatter script, inline template scripts, and tags.
pub(in crate::adapters) fn validate(
    _path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if source.len() > MAX_ASTRO {
        return Err(("PARSER_LIMIT", "Astro component parser limited to 2 MiB"));
    }
    context.check()?;

    let mut frontmatter_checked = false;
    let template_start =
        if let Some((body_start, body_end, after_fence)) = find_frontmatter(source)? {
            let body = &source[body_start..body_end];
            ts_syntax::validate_syntax("component.astro.ts", body, context)?;
            frontmatter_checked = true;
            after_fence
        } else {
            0
        };

    // Scan template for inline <script> and <style> tags
    let mut cursor = template_start;
    let mut scripts_checked = 0usize;
    let mut last_check = cursor;

    while cursor < source.len() {
        if cursor.saturating_sub(last_check) >= 32 * 1024 {
            context.check()?;
            last_check = cursor;
        }
        let Some(start) = source[cursor..].find('<').map(|n| cursor + n) else {
            break;
        };
        // Skip HTML comments <!-- ... -->
        if source[start..].starts_with("<!--") {
            cursor = source[start + 4..]
                .find("-->")
                .map(|n| start + 4 + n + 3)
                .ok_or(("SYNTAX_ERROR", "Unterminated comment in Astro template"))?;
            continue;
        }
        let Some(tag) = lex::parse_tag(source, start) else {
            cursor = start + 1;
            continue;
        };
        cursor = tag.end;
        if tag.closing {
            continue;
        }
        match tag.name.to_ascii_lowercase().as_str() {
            "script" if tag.self_closing => {
                // External or self-closing script
            }
            "script" => {
                let (body_end, after) = lex::closing_tag(source, cursor, "script").ok_or((
                    "SYNTAX_ERROR",
                    "Astro template <script> block is not closed",
                ))?;
                let body = &source[cursor..body_end];
                if !lex::has_attribute(tag.attrs, "src") {
                    let is_inline = lex::has_attribute(tag.attrs, "is:inline");
                    let virtual_path = if is_inline {
                        "script.astro.js"
                    } else {
                        "script.astro.ts"
                    };
                    ts_syntax::validate_syntax(virtual_path, body, context)?;
                    scripts_checked += 1;
                }
                cursor = after;
            }
            "style" if !tag.self_closing => {
                let (_, after) = lex::closing_tag(source, cursor, "style")
                    .ok_or(("SYNTAX_ERROR", "Astro template <style> block is not closed"))?;
                cursor = after;
            }
            _ => {}
        }
    }

    context.check()?;
    Ok(json!({
        "status": "valid",
        "language": "astro",
        "validation": "syntax_only",
        "frontmatterChecked": frontmatter_checked,
        "scriptsChecked": scripts_checked,
    }))
}

/// Collect declarations, imports, and types from an Astro component's frontmatter.
pub(in crate::adapters) fn collect_symbols(
    _path: &str,
    source: &str,
    digest: &str,
    context: &OperationContext,
) -> Result<Vec<Symbol>, Failure> {
    context.check()?;
    let Some((body_start, body_end, _)) = find_frontmatter(source)? else {
        return Ok(Vec::new());
    };
    let body = &source[body_start..body_end];
    if body.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut symbols = ts_symbols::collect("component.astro.ts", body, digest, context)?;
    for item in &mut symbols {
        item.bytes = body_start + item.bytes.start..body_start + item.bytes.end;
        item.start = line(source, item.bytes.start);
        item.end = line(source, item.bytes.end.saturating_sub(1));
        item.container = if item.container.is_empty() {
            "frontmatter".into()
        } else {
            format!("frontmatter/{}", item.container)
        };
        item.id = hash(
            format!(
                "{digest}:{}:{}:{}",
                item.kind, item.bytes.start, item.bytes.end
            )
            .as_bytes(),
        )[..24]
            .to_owned();
    }

    // Also collect top-level imports in frontmatter for direct dependency visibility
    let allocator = Allocator::default();
    let ast = Parser::new(
        &allocator,
        body,
        SourceType::from_path("component.astro.ts").unwrap_or_default(),
    )
    .parse();

    for stmt in &ast.program.body {
        context.check()?;
        if let Statement::ImportDeclaration(decl) = stmt {
            let range = body_start + decl.span.start as usize..body_start + decl.span.end as usize;
            let id =
                hash(format!("{digest}:astro_import:{}:{}", range.start, range.end).as_bytes())
                    [..24]
                    .to_owned();
            symbols.push(Symbol {
                name: format!("import {}", decl.source.value),
                container: "frontmatter".into(),
                kind: "astro_import",
                start: line(source, range.start),
                end: line(source, range.end.saturating_sub(1)),
                signature: prefix(&source[range.clone()], 400).to_owned(),
                attributes: Vec::new(),
                doc: String::new(),
                id,
                bytes: range,
            });
        }
    }

    Ok(symbols)
}

#[cfg(test)]
#[path = "syntax_tests.rs"]
mod tests;
