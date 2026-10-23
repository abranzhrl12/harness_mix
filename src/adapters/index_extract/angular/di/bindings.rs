//! Oxc-backed proof that a DI token is a live value import at this source site.
use oxc_ast::{
    AstKind,
    ast::{Expression, ModuleExportName},
};
use oxc_semantic::Semantic;
use oxc_span::GetSpan;

fn has_core_import(
    semantic: &Semantic<'_>,
    name: &str,
    start: u32,
    end: u32,
    imported: &str,
) -> bool {
    let Some(symbol) = semantic
        .scoping()
        .get_binding(semantic.scoping().root_scope_id(), name.into())
    else {
        return false;
    };
    if !semantic.scoping().symbol_redeclarations(symbol).is_empty()
        || semantic.scoping().symbol_flags(symbol).is_type_import()
        || !matches!(
            semantic.symbol_declaration(symbol).kind(),
            AstKind::ImportSpecifier(_)
        )
    {
        return false;
    }
    let declaration = semantic.symbol_declaration(symbol);
    let AstKind::ImportSpecifier(specifier) = declaration.kind() else {
        return false;
    };
    if specifier.import_kind != oxc_ast::ast::ImportOrExportKind::Value
        || module_export_name(&specifier.imported) != imported
    {
        return false;
    }
    semantic
        .symbol_references(symbol)
        .any(|reference| {
            let span = semantic
                .nodes()
                .get_node(reference.node_id())
                .kind()
                .span();
            span.start == start && span.end == end
        })
        && semantic.nodes().ancestors(declaration.id()).any(|node| {
            matches!(node.kind(), AstKind::ImportDeclaration(decl) if decl.source.value == "@angular/core")
        })
}

fn has_core_namespace_import(semantic: &Semantic<'_>, name: &str, start: u32, end: u32) -> bool {
    let Some(symbol) = semantic
        .scoping()
        .get_binding(semantic.scoping().root_scope_id(), name.into())
    else {
        return false;
    };
    if !semantic.scoping().symbol_redeclarations(symbol).is_empty()
        || !matches!(
            semantic.symbol_declaration(symbol).kind(),
            AstKind::ImportNamespaceSpecifier(_)
        )
    {
        return false;
    }
    let declaration = semantic.symbol_declaration(symbol);
    let AstKind::ImportNamespaceSpecifier(_) = declaration.kind() else {
        return false;
    };
    semantic.symbol_references(symbol).any(|reference| {
        let span = semantic
            .nodes()
            .get_node(reference.node_id())
            .kind()
            .span();
        span.start == start && span.end == end
    }) && semantic.nodes().ancestors(declaration.id()).any(|node| {
        matches!(node.kind(), AstKind::ImportDeclaration(decl) if decl.source.value == "@angular/core")
    })
}

fn module_export_name<'a>(name: &'a ModuleExportName<'a>) -> &'a str {
    match name {
        ModuleExportName::IdentifierName(identifier) => identifier.name.as_str(),
        ModuleExportName::IdentifierReference(identifier) => identifier.name.as_str(),
        ModuleExportName::StringLiteral(literal) => literal.value.as_str().unwrap_or(""),
    }
}

pub(super) fn is_core_decorator(
    semantic: &Semantic<'_>,
    expression: &Expression<'_>,
    roles: &[&str],
) -> bool {
    let Expression::CallExpression(call) = expression else {
        return false;
    };
    match &call.callee {
        Expression::Identifier(callee) => roles.iter().any(|role| {
            has_core_import(
                semantic,
                callee.name.as_str(),
                callee.span.start,
                callee.span.end,
                role,
            )
        }),
        Expression::StaticMemberExpression(member) => {
            let Expression::Identifier(namespace) = &member.object else {
                return false;
            };
            roles.contains(&member.property.name.as_str())
                && has_core_namespace_import(
                    semantic,
                    namespace.name.as_str(),
                    namespace.span.start,
                    namespace.span.end,
                )
        }
        _ => false,
    }
}

pub(super) fn imported_token(
    semantic: &Semantic<'_>,
    name: &str,
    start: u32,
    end: u32,
) -> Option<(String, String)> {
    let symbol = semantic
        .scoping()
        .get_binding(semantic.scoping().root_scope_id(), name.into())?;
    if !semantic.scoping().symbol_redeclarations(symbol).is_empty()
        || semantic.scoping().symbol_flags(symbol).is_type_import()
    {
        return None;
    }
    let declaration = semantic.symbol_declaration(symbol);
    let AstKind::ImportSpecifier(specifier) = declaration.kind() else {
        return None;
    };
    if specifier.import_kind != oxc_ast::ast::ImportOrExportKind::Value
        || !semantic.symbol_references(symbol).any(|reference| {
            let span = semantic.nodes().get_node(reference.node_id()).kind().span();
            span.start == start && span.end == end
        })
    {
        return None;
    }
    let imported = module_export_name(&specifier.imported).to_owned();
    let module = semantic
        .nodes()
        .ancestors(declaration.id())
        .find_map(|node| match node.kind() {
            AstKind::ImportDeclaration(declaration) => Some(declaration.source.value.display().to_string()),
            _ => None,
        })?;
    Some((module, imported))
}

pub(super) fn is_core_inject(semantic: &Semantic<'_>, name: &str, start: u32, end: u32) -> bool {
    has_core_import(semantic, name, start, end, "inject")
}
