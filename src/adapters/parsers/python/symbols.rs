//! Python AST declaration symbol collection using tree-sitter via ast-grep.
//! Extracts class_definition and function_definition symbols with signatures and UTF-16 symbolIds.

use super::syntax::DEFAULT_SOURCE_LIMIT_BYTES;
use crate::adapters::parsers::model::Symbol;
use ast_grep_language::SupportLang;
use harness_domain::{Failure, execution::OperationContext, hash, results::prefix};

pub(in crate::adapters) fn collect(
    source: &str,
    digest: &str,
    context: &OperationContext,
) -> Result<Vec<Symbol>, Failure> {
    collect_with_limit(source, digest, context, DEFAULT_SOURCE_LIMIT_BYTES)
}

pub(in crate::adapters) fn collect_with_limit(
    source: &str,
    digest: &str,
    context: &OperationContext,
    max_source_bytes: usize,
) -> Result<Vec<Symbol>, Failure> {
    context.check()?;
    if source.len() > max_source_bytes {
        return Err((
            "PARSER_LIMIT",
            if max_source_bytes == DEFAULT_SOURCE_LIMIT_BYTES {
                "Python source exceeds 512 KiB limit"
            } else {
                "Python source exceeds the configured symbol parsing limit"
            },
        ));
    }

    let tree = ast_grep_core::AstGrep::try_new(source, SupportLang::Python)
        .map_err(|_| ("SYNTAX_ERROR", "Python syntax error"))?;
    context.check()?;
    if tree.root().get_doc().tree.root_node().has_error() {
        return Err(("SYNTAX_ERROR", "Python syntax error"));
    }

    let mut symbols = Vec::new();
    let root = tree.root();
    walk_node(&root, "", false, source, digest, context, &mut symbols)?;
    Ok(symbols)
}

fn walk_node<'a, D: ast_grep_core::Doc>(
    parent: &ast_grep_core::Node<'a, D>,
    container: &str,
    in_class: bool,
    source: &str,
    digest: &str,
    context: &OperationContext,
    symbols: &mut Vec<Symbol>,
) -> Result<(), Failure> {
    for child in parent.children() {
        context.check()?;
        let kind = child.kind();
        match kind.as_ref() {
            "decorated_definition" => {
                if let Some(def_node) = child.field("definition") {
                    record_definition(
                        &def_node,
                        Some(&child),
                        container,
                        in_class,
                        source,
                        digest,
                        context,
                        symbols,
                    )?;
                }
            }
            "class_definition" | "function_definition" => {
                record_definition(
                    &child,
                    None,
                    container,
                    in_class,
                    source,
                    digest,
                    context,
                    symbols,
                )?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_definition<'a, D: ast_grep_core::Doc>(
    def_node: &ast_grep_core::Node<'a, D>,
    decorated_outer: Option<&ast_grep_core::Node<'a, D>>,
    container: &str,
    in_class: bool,
    source: &str,
    digest: &str,
    context: &OperationContext,
    symbols: &mut Vec<Symbol>,
) -> Result<(), Failure> {
    context.check()?;
    let raw_kind = def_node.kind();
    let is_class = raw_kind.as_ref() == "class_definition";
    let symbol_kind = if is_class {
        "class"
    } else if in_class {
        "method"
    } else {
        "function"
    };

    let Some(name_node) = def_node.field("name") else {
        return Ok(());
    };
    let name_range = name_node.range();
    let Some(name_slice) = source.get(name_range) else {
        return Err(("RANGE", "Parser offsets incompatible with source"));
    };
    let name = name_slice.trim();
    if name.is_empty() {
        return Ok(());
    }
    if symbols.len() >= 1000 || name.len() > 1024 || container.len() > 4096 {
        return Err(("PARSER_LIMIT", "Symbol count or identifier budget exceeded"));
    }

    let span_node = decorated_outer.unwrap_or(def_node);
    let full_range = span_node.range();
    let start_u = full_range.start;
    let end_u = full_range.end;
    if start_u >= end_u || source.get(start_u..end_u).is_none() {
        return Err(("RANGE", "Parser offsets incompatible with source"));
    }

    let body_node = def_node.field("body");
    let header_end = body_node
        .as_ref()
        .map(|b| b.range().start.min(end_u))
        .unwrap_or(end_u);
    let sig_slice = source
        .get(start_u..header_end)
        .unwrap_or(&source[start_u..end_u]);
    let compact = sig_slice.split_whitespace().collect::<Vec<_>>().join(" ");
    let signature = prefix(&compact, 400).to_owned();

    let mut attributes = Vec::new();
    if let Some(outer) = decorated_outer {
        for child in outer.children() {
            if child.kind().as_ref() == "decorator" {
                if let Some(dec_text) = source.get(child.range()) {
                    attributes.push(dec_text.trim().to_owned());
                }
            }
        }
    }

    let doc = extract_docstring(body_node.as_ref(), source);

    let full_text = &source[start_u..end_u];
    let from = source[..start_u].encode_utf16().count();
    let to = from + full_text.encode_utf16().count();
    let id = hash(format!("{digest}:{from}:{to}").as_bytes())[..24].to_owned();

    let start_line = source[..start_u].bytes().filter(|b| *b == b'\n').count() + 1;
    let end_line = source[..end_u].bytes().filter(|b| *b == b'\n').count()
        + usize::from(!source[..end_u].ends_with('\n'));

    let next_container = if container.is_empty() {
        name.to_owned()
    } else {
        format!("{container}::{name}")
    };

    symbols.push(Symbol {
        name: name.to_owned(),
        container: container.to_owned(),
        kind: symbol_kind,
        start: start_line,
        end: end_line,
        signature,
        attributes,
        doc: prefix(&doc, 600).to_owned(),
        id,
        bytes: start_u..end_u,
    });

    if let Some(body) = body_node {
        walk_node(
            &body,
            &next_container,
            is_class,
            source,
            digest,
            context,
            symbols,
        )?;
    }

    Ok(())
}

fn extract_docstring<'a, D: ast_grep_core::Doc>(
    body: Option<&ast_grep_core::Node<'a, D>>,
    source: &str,
) -> String {
    let Some(body) = body else {
        return String::new();
    };
    let Some(first_stmt) = body.children().next() else {
        return String::new();
    };
    if first_stmt.kind().as_ref() != "expression_statement" {
        return String::new();
    }
    let Some(expr) = first_stmt.children().next() else {
        return String::new();
    };
    if expr.kind().as_ref() != "string" {
        return String::new();
    }
    let Some(raw) = source.get(expr.range()) else {
        return String::new();
    };
    let trimmed = raw
        .trim()
        .trim_start_matches(['r', 'R', 'u', 'U', 'f', 'F', 'b', 'B']);
    for quote in ["\"\"\"", "'''", "\"", "'"] {
        if let Some(inner) = trimmed
            .strip_prefix(quote)
            .and_then(|s| s.strip_suffix(quote))
        {
            return inner.trim().to_owned();
        }
    }
    String::new()
}
