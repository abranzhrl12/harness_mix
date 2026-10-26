//! Conservative bindings and declarations for the Rust graph subset.
use super::{Facts, position};
use harness_domain::{Failure, execution::OperationContext};
use ra_ap_syntax::{
    SyntaxKind, SyntaxNode,
    ast::{self, AstNode, HasAttrs, HasDocComments, HasName, HasVisibility},
};
use std::collections::BTreeSet;
const SYNTAX_ERROR: Failure = ("SYNTAX_ERROR", "Rust source is not syntactically valid");
pub(super) fn collect_top_level(
    root: &SyntaxNode,
    source: &str,
    base: usize,
    context: &OperationContext,
    facts: &mut Facts,
) -> Result<(), Failure> {
    for child in root.children() {
        context.check()?;
        if let Some(function) = ast::Fn::cast(child.clone()) {
            if only_doc_attrs(&function) {
                let Some(name) = function.name() else {
                    continue;
                };
                let line = position(
                    source,
                    usize::from(
                        function
                            .fn_token()
                            .ok_or(SYNTAX_ERROR)?
                            .text_range()
                            .start(),
                    ) + base,
                )
                .ok_or(("RANGE", "Invalid Rust function position"))?
                .0;
                let visibility = function.visibility().map(|v| token_text(v.syntax()));
                let public = matches!(
                    visibility.as_deref(),
                    Some("pub" | "pub(crate)" | "pub(super)" | "pub(incrate)" | "pub(insuper)")
                );
                facts
                    .functions
                    .insert(name.text().to_string(), (line, public));
            }
        } else if let Some(module) = ast::Module::cast(child) {
            let attrs_match = module.attrs().all(|attr| {
                cfg!(windows)
                    && attr.simple_name().is_some_and(|name| name == "cfg")
                    && attr
                        .meta()
                        .and_then(|m| m.token_tree())
                        .is_some_and(|tree| token_text(tree.syntax()) == "(windows)")
            });
            if attrs_match && module.doc_comments().next().is_none() && module.item_list().is_none()
            {
                if let Some(name) = module.name() {
                    facts.modules.insert(name.text().to_string());
                }
            }
        }
    }
    Ok(())
}

pub(super) fn only_doc_attrs(function: &ast::Fn) -> bool {
    function
        .attrs()
        .all(|attr| attr.simple_name().is_some_and(|name| name == "doc"))
}

fn token_text(node: &SyntaxNode) -> String {
    node.descendants_with_tokens()
        .filter_map(|n| n.into_token())
        .filter(|token| !token.kind().is_trivia())
        .map(|token| token.text().to_owned())
        .collect()
}

pub(super) fn bindings(root: &SyntaxNode, context: &OperationContext) -> Result<Bindings, Failure> {
    let mut result = Bindings::default();
    collect_bindings(root, context, &mut result)?;
    Ok(result)
}

#[derive(Default)]
pub(super) struct Bindings {
    pub(super) imported_names: BTreeSet<String>,
    pub(super) glob_import: bool,
    pub(super) ambiguous_module_macro: bool,
}

fn collect_bindings(
    node: &SyntaxNode,
    context: &OperationContext,
    out: &mut Bindings,
) -> Result<(), Failure> {
    context.check()?;
    for child in node.children() {
        context.check()?;
        if child.kind() == SyntaxKind::MODULE || child.kind() == SyntaxKind::FN {
            continue;
        }
        match child.kind() {
            SyntaxKind::USE => {
                for element in child.descendants_with_tokens() {
                    let Some(token) = element.as_token() else {
                        continue;
                    };
                    if token.text() == "*" {
                        out.glob_import = true;
                    } else if token.kind() == SyntaxKind::IDENT {
                        out.imported_names.insert(token.text().to_string());
                    }
                }
            }
            SyntaxKind::MACRO_CALL => {
                if !macro_is_expression_context(&child) {
                    out.ambiguous_module_macro = true;
                }
            }
            _ => {}
        }
        collect_bindings(&child, context, out)?;
    }
    Ok(())
}

pub(super) fn macro_is_expression_context(node: &SyntaxNode) -> bool {
    for ancestor in node.ancestors().skip(1) {
        match ancestor.kind() {
            SyntaxKind::CALL_EXPR | SyntaxKind::METHOD_CALL_EXPR => return true,
            SyntaxKind::LET_STMT => {
                let in_initializer = ast::LetStmt::cast(ancestor.clone())
                    .and_then(|statement| statement.initializer())
                    .is_some_and(|initializer| {
                        let range = initializer.syntax().text_range();
                        range.start() <= node.text_range().start()
                            && range.end() >= node.text_range().end()
                    });
                if in_initializer {
                    return true;
                }
            }
            SyntaxKind::RETURN_EXPR => {
                let in_return_value = ast::ReturnExpr::cast(ancestor.clone())
                    .and_then(|expression| expression.expr())
                    .is_some_and(|value| {
                        let range = value.syntax().text_range();
                        range.start() <= node.text_range().start()
                            && range.end() >= node.text_range().end()
                    });
                if in_return_value {
                    return true;
                }
            }
            SyntaxKind::MATCH_ARM => {
                let in_arm_expression = ast::MatchArm::cast(ancestor.clone())
                    .and_then(|arm| arm.expr())
                    .is_some_and(|expression| {
                        let range = expression.syntax().text_range();
                        range.start() <= node.text_range().start()
                            && range.end() >= node.text_range().end()
                    });
                if in_arm_expression {
                    return true;
                }
            }
            SyntaxKind::FN | SyntaxKind::STMT_LIST | SyntaxKind::MODULE => break,
            _ => {}
        }
    }
    false
}
