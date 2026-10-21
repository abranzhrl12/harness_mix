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
    collect_bindings(root, true, context, &mut result)?;
    Ok(result)
}

#[derive(Default)]
pub(super) struct Bindings {
    pub(super) names: BTreeSet<String>,
    pub(super) imports: bool,
}

fn collect_bindings(
    node: &SyntaxNode,
    _root: bool,
    context: &OperationContext,
    out: &mut Bindings,
) -> Result<(), Failure> {
    context.check()?;
    for child in node.children() {
        context.check()?;
        if child.kind() == SyntaxKind::MODULE {
            continue;
        }
        match child.kind() {
            SyntaxKind::IDENT_PAT => {
                if let Some(name) = ast::IdentPat::cast(child.clone()).and_then(|p| p.name()) {
                    out.names.insert(name.text().to_string());
                }
            }
            SyntaxKind::USE | SyntaxKind::MACRO_CALL | SyntaxKind::MACRO_RULES => {
                out.imports = true
            }
            SyntaxKind::FN | SyntaxKind::CONST | SyntaxKind::STATIC
                if child
                    .parent()
                    .is_some_and(|p| p.kind() == SyntaxKind::STMT_LIST) =>
            {
                let name = match child.kind() {
                    SyntaxKind::FN => ast::Fn::cast(child.clone()).and_then(|item| item.name()),
                    SyntaxKind::CONST => {
                        ast::Const::cast(child.clone()).and_then(|item| item.name())
                    }
                    SyntaxKind::STATIC => {
                        ast::Static::cast(child.clone()).and_then(|item| item.name())
                    }
                    _ => None,
                };
                if let Some(name) = name {
                    out.names.insert(name.text().to_string());
                }
            }
            _ => {}
        }
        collect_bindings(&child, false, context, out)?;
    }
    Ok(())
}
