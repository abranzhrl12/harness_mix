//! Call candidates preserve the existing no-receiver/type-guessing policy.
use super::{
    bindings::{Bindings, macro_is_expression_context, only_doc_attrs},
    position,
};
use harness_domain::{Failure, execution::OperationContext};
use ra_ap_syntax::{
    SyntaxKind, SyntaxNode,
    ast::{self, AstNode, HasName},
};
use std::collections::{BTreeMap, BTreeSet};
pub(super) fn visit_calls(
    node: &SyntaxNode,
    source: &str,
    base: usize,
    context: &OperationContext,
    bindings: &Bindings,
    functions: &BTreeSet<String>,
    receivers: &mut BTreeSet<(String, i64, i64)>,
    calls: &mut BTreeMap<(String, i64, i64), Vec<String>>,
) -> Result<(), Failure> {
    context.check()?;
    for child in node.children() {
        context.check()?;
        match child.kind() {
            SyntaxKind::MODULE | SyntaxKind::IMPL | SyntaxKind::TRAIT => continue,
            SyntaxKind::FN => {
                let Some(function) = ast::Fn::cast(child.clone()) else {
                    continue;
                };
                if !only_doc_attrs(&function) {
                    continue;
                }
            }
            SyntaxKind::METHOD_CALL_EXPR => {
                if let Some(method) = ast::MethodCallExpr::cast(child.clone()) {
                    if let Some(name) = method.name_ref() {
                        let at = usize::from(child.text_range().start()) + base;
                        if let Some((line, col)) = position(source, at) {
                            receivers.insert((name.text().to_string(), line, col));
                        }
                    }
                }
            }
            SyntaxKind::CALL_EXPR => {
                if let Some(call) = ast::CallExpr::cast(child.clone()) {
                    let path_expr = call.expr().and_then(|expr| match expr {
                        ast::Expr::PathExpr(path_expr) => Some(path_expr),
                        _ => None,
                    });
                    if let Some(path_expr) = path_expr {
                        if let Some(path) = path_expr.path() {
                            if !has_qualified_self(&path_expr.syntax()) {
                                let path_text = path.syntax().text().to_string();
                                if !path_text.starts_with("::") {
                                    let parts = path_parts(path);
                                    if let Some(first) = parts.first() {
                                        let fallback_is_unshadowed = if parts.len() > 1 {
                                            true
                                        } else {
                                            let (local_name, local_glob, local_macro) =
                                                local_scope_state(&path_expr.syntax(), first);
                                            !bindings.imported_names.contains(first)
                                                && !bindings.glob_import
                                                && !bindings.ambiguous_module_macro
                                                && !local_name
                                                && !local_glob
                                                && !local_macro
                                        };
                                        if fallback_is_unshadowed
                                            && (parts.len() > 1 || functions.contains(first))
                                        {
                                            let at = usize::from(
                                                path_expr.syntax().text_range().start(),
                                            ) + base;
                                            if let Some((line, col)) = position(source, at) {
                                                calls.insert((parts.join("::"), line, col), parts);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        visit_calls(
            &child, source, base, context, bindings, functions, receivers, calls,
        )?;
    }
    Ok(())
}

fn local_scope_state(node: &SyntaxNode, name: &str) -> (bool, bool, bool) {
    let Some(function) = node
        .ancestors()
        .find(|ancestor| ancestor.kind() == SyntaxKind::FN)
    else {
        return (false, false, false);
    };

    let mut shadowed = false;
    let mut glob_import = false;
    let mut ambiguous_macro = false;
    // This union is intentionally broader than lexical Rust scopes. It can
    // suppress a sibling-block fallback, but never treats an uncertain local
    // binding as proof of a project target.
    for descendant in function.descendants() {
        match descendant.kind() {
            SyntaxKind::FN => {
                if ast::Fn::cast(descendant.clone())
                    .and_then(|item| item.name())
                    .is_some_and(|binding| binding.text() == name)
                {
                    shadowed = true;
                }
            }
            SyntaxKind::CONST => {
                if ast::Const::cast(descendant.clone())
                    .and_then(|item| item.name())
                    .is_some_and(|binding| binding.text() == name)
                {
                    shadowed = true;
                }
            }
            SyntaxKind::STATIC => {
                if ast::Static::cast(descendant.clone())
                    .and_then(|item| item.name())
                    .is_some_and(|binding| binding.text() == name)
                {
                    shadowed = true;
                }
            }
            SyntaxKind::IDENT_PAT => {
                if ast::IdentPat::cast(descendant.clone())
                    .and_then(|pattern| pattern.name())
                    .is_some_and(|binding| binding.text() == name)
                {
                    shadowed = true;
                }
            }
            SyntaxKind::USE => {
                for element in descendant.descendants_with_tokens() {
                    let Some(token) = element.as_token() else {
                        continue;
                    };
                    if token.text() == "*" {
                        glob_import = true;
                    } else if token.kind() == SyntaxKind::IDENT && token.text() == name {
                        shadowed = true;
                    }
                }
            }
            SyntaxKind::MACRO_CALL if !macro_is_expression_context(&descendant) => {
                ambiguous_macro = true;
            }
            _ => {}
        }
    }
    (shadowed, glob_import, ambiguous_macro)
}

fn has_qualified_self(node: &SyntaxNode) -> bool {
    let text = node.text().to_string();
    text.trim_start().starts_with('<') || text.contains(" as ")
}

fn path_parts(path: ast::Path) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = Some(path);
    while let Some(path) = current {
        if let Some(segment) = path.segment() {
            if let Some(name) = segment.name_ref() {
                parts.push(name.text().to_string());
            }
        }
        current = path.qualifier();
    }
    parts.reverse();
    parts
}
