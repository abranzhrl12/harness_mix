//! RA declaration navigation; no macro expansion or call inference.
use super::super::model::Symbol;
use crate::adapters::parsers::rust::syntax as rust_syntax;
use harness_domain::{Failure, execution::OperationContext, hash, results::prefix};
use ra_ap_syntax::{AstNode, AstToken, SyntaxKind, SyntaxNode, ast};

pub(in crate::adapters) fn collect(
    source: &str,
    digest: &str,
    context: &OperationContext,
) -> Result<Vec<Symbol>, Failure> {
    rust_syntax::with_ast(source, context, |file, base| {
        let mut output = Vec::new();
        walk(
            file.syntax(),
            "",
            source,
            base,
            digest,
            context,
            &mut output,
        )?;
        Ok(output)
    })
}

fn walk(
    parent: &SyntaxNode,
    container: &str,
    source: &str,
    base: usize,
    digest: &str,
    context: &OperationContext,
    output: &mut Vec<Symbol>,
) -> Result<(), Failure> {
    use SyntaxKind::*;
    for node in parent.children() {
        context.check()?;
        let kind = match node.kind() {
            FN => {
                if parent.kind() == ASSOC_ITEM_LIST
                    && parent.parent().is_some_and(|n| n.kind() == TRAIT)
                    && !node.children().any(|n| n.kind() == BLOCK_EXPR)
                {
                    "function_signature_item"
                } else {
                    "function_item"
                }
            }
            STRUCT => "struct_item",
            ENUM => "enum_item",
            UNION => "union_item",
            TRAIT | TRAIT_ALIAS => "trait_item",
            MODULE => "mod_item",
            TYPE_ALIAS => "type_item",
            CONST => "const_item",
            STATIC => "static_item",
            MACRO_RULES => "macro_definition",
            IMPL => "impl_item",
            _ => continue,
        };
        if output.len() >= 1000 {
            return Err(("PARSER_LIMIT", "Symbol budget exceeded"));
        }
        let name = if let Some(item) = ast::Impl::cast(node.clone()) {
            let ty = item
                .self_ty()
                .ok_or(("RANGE", "Missing self type"))?
                .syntax()
                .text()
                .to_string();
            if let Some(trait_) = item.trait_() {
                let negative = node
                    .children_with_tokens()
                    .any(|e| e.as_token().is_some_and(|t| t.text() == "!"));
                format!(
                    "impl {}{} for {ty}",
                    if negative { "!" } else { "" },
                    trait_.syntax().text()
                )
            } else {
                format!("impl {ty}")
            }
        } else {
            node.children()
                .find_map(ast::Name::cast)
                .ok_or(("RANGE", "Missing declaration name"))?
                .text()
                .to_string()
        };
        if name.len() > 1024 || container.len() > 4096 {
            return Err(("PARSER_LIMIT", "Symbol count or identifier budget exceeded"));
        }
        let range = node.text_range();
        // Rowan attaches ordinary leading comments to declarations; syn does not.
        let start = node
            .children_with_tokens()
            .find(|element| match element {
                ra_ap_syntax::NodeOrToken::Node(_) => true,
                ra_ap_syntax::NodeOrToken::Token(t) => {
                    t.kind() != WHITESPACE
                        && !ast::Comment::cast(t.clone()).is_some_and(|c| !c.is_doc())
                }
            })
            .map(|e| usize::from(e.text_range().start()))
            .unwrap_or(usize::from(range.start()));
        let bytes = start + base..usize::from(range.end()) + base;
        let mut signature_start = bytes.start;
        let mut docs = Vec::new();
        for element in node.children_with_tokens() {
            match element {
                ra_ap_syntax::NodeOrToken::Node(n) if n.kind() == ATTR => {
                    signature_start = usize::from(n.text_range().end()) + base;
                    if ast::Attr::cast(n.clone())
                        .and_then(|a| a.simple_name())
                        .is_some_and(|name| name == "doc")
                    {
                        if let Some(ast::Expr::Literal(literal)) = ast::Attr::cast(n)
                            .and_then(|a| a.meta())
                            .and_then(|m| m.expr())
                        {
                            if let ast::LiteralKind::String(value) = literal.kind() {
                                if let Ok(value) = value.value() {
                                    docs.push(value.into_owned());
                                }
                            }
                        }
                    }
                }
                ra_ap_syntax::NodeOrToken::Token(t) => {
                    if let Some(comment) = ast::Comment::cast(t) {
                        if let Some(doc) = comment.doc_comment() {
                            signature_start =
                                usize::from(comment.syntax().text_range().end()) + base;
                            docs.push(doc.replace("\r\n", "\n").trim_end_matches('\r').to_owned());
                        }
                    }
                }
                _ => {}
            }
        }
        let body = node.children().find(|n| {
            matches!(
                n.kind(),
                BLOCK_EXPR
                    | RECORD_FIELD_LIST
                    | VARIANT_LIST
                    | ASSOC_ITEM_LIST
                    | ITEM_LIST
                    | TOKEN_TREE
            )
        });
        let end = body
            .as_ref()
            .map(|n| usize::from(n.text_range().start()) + base)
            .unwrap_or(bytes.end);
        let compact = source
            .get(signature_start..end)
            .ok_or(("RANGE", "Invalid signature"))?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let signature = if body.is_some() {
            format!("{} …", prefix(&compact, 398))
        } else {
            prefix(&compact, 400).to_owned()
        };
        let from = source[..bytes.start].encode_utf16().count();
        let to = from + source[bytes.clone()].encode_utf16().count();
        output.push(Symbol {
            name: name.clone(),
            container: container.to_owned(),
            kind,
            start: source[..bytes.start]
                .bytes()
                .filter(|b| *b == b'\n')
                .count()
                + 1,
            end: source[..bytes.end].bytes().filter(|b| *b == b'\n').count()
                + usize::from(!source[..bytes.end].ends_with('\n')),
            signature,
            attributes: Vec::new(),
            doc: prefix(&docs.join("\n"), 600).to_owned(),
            id: hash(format!("{digest}:{from}:{to}").as_bytes())[..24].to_owned(),
            bytes,
        });
        if matches!(node.kind(), MODULE | IMPL | TRAIT) {
            let child = if container.is_empty() {
                name
            } else {
                format!("{container}::{name}")
            };
            if let Some(list) = node
                .children()
                .find(|n| matches!(n.kind(), ITEM_LIST | ASSOC_ITEM_LIST))
            {
                walk(&list, &child, source, base, digest, context, output)?;
            }
        }
    }
    Ok(())
}
