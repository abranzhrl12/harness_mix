//! Call candidates preserve the existing no-receiver/type-guessing policy.
use super::{
    bindings::{Bindings, only_doc_attrs},
    position,
};
use harness_domain::{Failure, execution::OperationContext};
use ra_ap_syntax::{
    SyntaxKind, SyntaxNode,
    ast::{self, AstNode},
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
                                        if !bindings.names.contains(first)
                                            && (parts.len() > 1
                                                || (!bindings.imports && functions.contains(first)))
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
