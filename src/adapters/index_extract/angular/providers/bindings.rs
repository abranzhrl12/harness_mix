//! Oxc lexical import and decorator proof for Angular provider metadata.
use oxc_ast::{
    AstKind,
    ast::{
        Class, Declaration, Expression, ModuleExportName, ObjectExpression, ObjectPropertyKind,
        PropertyKey, Statement,
    },
};
use oxc_semantic::Semantic;
use oxc_span::GetSpan;
use std::collections::BTreeMap;

#[derive(Clone)]
pub(super) struct ImportBinding {
    pub module: String,
    pub imported: String,
}

pub(super) fn imported_value(
    semantic: &Semantic<'_>,
    name: &str,
    start: u32,
    end: u32,
) -> Option<ImportBinding> {
    let symbol = semantic
        .scoping()
        .get_binding(semantic.scoping().root_scope_id(), name)?;
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
    let module = semantic
        .nodes()
        .ancestors(declaration.id())
        .find_map(|node| match node.kind() {
            AstKind::ImportDeclaration(decl) => Some(decl.source.value.to_string()),
            _ => None,
        })?;
    Some(ImportBinding {
        module,
        imported: imported_name(&specifier.imported).to_owned(),
    })
}

fn imported_name<'a>(name: &'a ModuleExportName<'a>) -> &'a str {
    match name {
        ModuleExportName::IdentifierName(id) => id.name.as_str(),
        ModuleExportName::IdentifierReference(id) => id.name.as_str(),
        ModuleExportName::StringLiteral(lit) => lit.value.as_str(),
    }
}

pub(super) fn decorator_role(
    semantic: &Semantic<'_>,
    expression: &Expression<'_>,
) -> Option<&'static str> {
    let Expression::CallExpression(call) = expression else {
        return None;
    };
    match &call.callee {
        Expression::Identifier(identifier) => ["Component", "Directive", "NgModule"]
            .into_iter()
            .find(|role| {
                imported_value(
                    semantic,
                    identifier.name.as_str(),
                    identifier.span.start,
                    identifier.span.end,
                )
                .is_some_and(|binding| {
                    binding.module == "@angular/core" && binding.imported == *role
                })
            }),
        Expression::StaticMemberExpression(member) => {
            let Expression::Identifier(namespace) = &member.object else {
                return None;
            };
            let symbol = semantic
                .scoping()
                .get_binding(semantic.scoping().root_scope_id(), namespace.name.as_str())?;
            if !semantic.scoping().symbol_redeclarations(symbol).is_empty()
                || semantic.scoping().symbol_flags(symbol).is_type_import()
                || !matches!(
                    semantic.symbol_declaration(symbol).kind(),
                    AstKind::ImportNamespaceSpecifier(_)
                )
            {
                return None;
            }
            let declaration = semantic.symbol_declaration(symbol);
            let is_core = semantic.symbol_references(symbol).any(|reference| {
                let span = semantic.nodes().get_node(reference.node_id()).kind().span();
                span.start == namespace.span.start && span.end == namespace.span.end
            }) && semantic.nodes().ancestors(declaration.id()).any(|node| {
                matches!(node.kind(), AstKind::ImportDeclaration(decl) if decl.source.value == "@angular/core")
            });
            let role = member.property.name.as_str();
            (is_core && ["Component", "Directive", "NgModule"].contains(&role)).then_some(
                match role {
                    "Component" => "Component",
                    "Directive" => "Directive",
                    _ => "NgModule",
                },
            )
        }
        _ => None,
    }
}

pub(super) fn classes<'a>(program: &'a oxc_ast::ast::Program<'a>) -> Vec<&'a Class<'a>> {
    program
        .body
        .iter()
        .filter_map(|statement| match statement {
            Statement::ClassDeclaration(class) => Some(class.as_ref()),
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::ClassDeclaration(class)) => Some(class.as_ref()),
                _ => None,
            },
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                    Some(class.as_ref())
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

pub(super) fn properties<'a>(
    object: &'a ObjectExpression<'a>,
) -> Result<BTreeMap<String, &'a oxc_ast::ast::ObjectProperty<'a>>, ()> {
    let mut found = BTreeMap::new();
    for entry in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = entry else {
            return Err(());
        };
        if property.computed || property.method {
            return Err(());
        }
        let key = match &property.key {
            PropertyKey::StaticIdentifier(id) => id.name.to_string(),
            PropertyKey::StringLiteral(literal) => literal.value.to_string(),
            _ => return Err(()),
        };
        if found.insert(key, &**property).is_some() {
            return Err(());
        }
    }
    Ok(found)
}
