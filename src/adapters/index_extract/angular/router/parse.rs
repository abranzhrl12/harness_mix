//! Oxc based static Angular route table recognition.
use harness_domain::{Failure, execution::OperationContext};
use oxc_ast::AstKind;
use oxc_ast::ast::{
    Argument, ArrayExpression, BindingPattern, BindingPatternKind, Class, Expression,
    ImportDeclarationSpecifier, ModuleExportName, ObjectPropertyKind, PropertyKey, Statement,
    TSType, TSTypeName, VariableDeclarationKind, VariableDeclarator,
};
use oxc_semantic::{Semantic, SemanticBuilder};
use oxc_span::GetSpan;
use oxc_syntax::symbol::SymbolId;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const MAX_ROUTES: usize = 2048;
const MAX_DEPTH: usize = 12;
const MAX_SCAN_ITEMS: usize = 20_000;
const LIMIT: Failure = (
    "EXTRACTION_LIMIT",
    "Angular route extraction limit exceeded",
);

#[derive(Default)]
struct RouterImports {
    value_bindings: BTreeMap<String, String>,
    namespaces: BTreeSet<String>,
    route_types: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Registration {
    Root,
    Child,
}

enum RouteArgument<'a> {
    Array(&'a ArrayExpression<'a>, Registration),
    Identifier(String, Registration, oxc_span::Span),
}

#[derive(Clone)]
pub(super) struct ComponentRef {
    pub(super) name: String,
    pub(super) at: usize,
    pub(super) kind: &'static str,
}

pub(super) struct RouteEntry {
    pub(super) path: String,
    pub(super) path_at: usize,
    pub(super) component: Option<ComponentRef>,
    pub(super) children: Vec<RouteEntry>,
}

pub(super) fn imported_name(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(id) => id.name.to_string(),
        ModuleExportName::IdentifierReference(id) => id.name.to_string(),
        ModuleExportName::StringLiteral(lit) => lit.value.to_string(),
    }
}

fn router_imports(program: &oxc_ast::ast::Program<'_>) -> RouterImports {
    let mut result = RouterImports::default();
    for statement in &program.body {
        let Statement::ImportDeclaration(declaration) = statement else {
            continue;
        };
        if declaration.source.value.as_str() != "@angular/router" {
            continue;
        }
        let Some(specifiers) = &declaration.specifiers else {
            continue;
        };
        for specifier in specifiers {
            match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                    let original = imported_name(&specifier.imported);
                    let local = specifier.local.name.to_string();
                    // Route(s) are type-only syntax gates, never runtime bindings.
                    if matches!(original.as_str(), "Route" | "Routes") {
                        result.route_types.insert(local.clone());
                    }
                    if declaration.import_kind == oxc_ast::ast::ImportOrExportKind::Value
                        && specifier.import_kind == oxc_ast::ast::ImportOrExportKind::Value
                    {
                        result.value_bindings.insert(local, original);
                    }
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier)
                    if declaration.import_kind == oxc_ast::ast::ImportOrExportKind::Value =>
                {
                    result.namespaces.insert(specifier.local.name.to_string());
                }
                _ => {}
            }
        }
    }
    result
}

fn is_router_module(expression: &Expression<'_>, imports: &RouterImports) -> bool {
    match expression {
        Expression::Identifier(identifier) => imports
            .value_bindings
            .get(identifier.name.as_str())
            .is_some_and(|name| name == "RouterModule"),
        Expression::StaticMemberExpression(member)
            if member.property.name.as_str() == "RouterModule" =>
        {
            matches!(&member.object, Expression::Identifier(id) if imports.namespaces.contains(id.name.as_str()))
        }
        _ => false,
    }
}

fn registered_call<'a>(
    expression: &'a Expression<'a>,
    imports: &RouterImports,
) -> Option<RouteArgument<'a>> {
    let Expression::CallExpression(call) = expression else {
        return None;
    };
    let registration = match &call.callee {
        Expression::Identifier(identifier)
            if imports
                .value_bindings
                .get(identifier.name.as_str())
                .is_some_and(|name| name == "provideRouter") =>
        {
            Registration::Root
        }
        Expression::StaticMemberExpression(member)
            if matches!(member.property.name.as_str(), "forRoot" | "forChild")
                && is_router_module(&member.object, imports) =>
        {
            if member.property.name.as_str() == "forRoot" {
                Registration::Root
            } else {
                Registration::Child
            }
        }
        _ => return None,
    };
    match call.arguments.first()? {
        Argument::ArrayExpression(array) => Some(RouteArgument::Array(array, registration)),
        Argument::Identifier(identifier) => Some(RouteArgument::Identifier(
            identifier.name.to_string(),
            registration,
            identifier.span,
        )),
        _ => None,
    }
}

fn static_path(source: &str, expression: &Expression<'_>) -> Option<(String, usize)> {
    let Expression::StringLiteral(value) = expression else {
        return None;
    };
    let span = expression.span();
    let start = span.start as usize;
    let raw = source.get(start..span.end as usize)?;
    let quote = *raw.as_bytes().first()?;
    if !matches!(quote, b'\'' | b'"')
        || raw.as_bytes().last().copied() != Some(quote)
        || raw.get(1..raw.len().checked_sub(1)?)?.contains('\\')
        || raw.get(1..raw.len() - 1)? != value.value.as_str()
    {
        return None;
    }
    Some((value.value.to_string(), start))
}

fn properties<'a>(
    object: &'a oxc_ast::ast::ObjectExpression<'a>,
) -> Option<BTreeMap<String, &'a Expression<'a>>> {
    let mut result = BTreeMap::new();
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else {
            return None;
        };
        if property.computed || property.method {
            return None;
        }
        let key = match &property.key {
            PropertyKey::StaticIdentifier(key) => key.name.to_string(),
            PropertyKey::StringLiteral(key) => key.value.to_string(),
            _ => return None,
        };
        if result.insert(key, &property.value).is_some() {
            return None;
        }
    }
    Some(result)
}

fn identifier_ref(expression: &Expression<'_>) -> Option<(String, usize)> {
    match expression {
        Expression::Identifier(identifier) => {
            Some((identifier.name.to_string(), identifier.span.start as usize))
        }
        _ => None,
    }
}

fn parse_array(
    source: &str,
    array: &ArrayExpression<'_>,
    depth: usize,
    budget: &mut usize,
    context: &OperationContext,
) -> Result<Vec<RouteEntry>, Failure> {
    if depth > MAX_DEPTH {
        return Err(LIMIT);
    }
    let mut result = Vec::new();
    for element in &array.elements {
        context.check()?;
        *budget += 1;
        if *budget > MAX_SCAN_ITEMS {
            return Err(LIMIT);
        }
        let Some(Expression::ObjectExpression(object)) = element.as_expression() else {
            continue;
        };
        let Some(fields) = properties(object) else {
            continue;
        };
        if fields.contains_key("redirectTo")
            || fields.contains_key("matcher")
            || fields.contains_key("loadChildren")
        {
            continue;
        }
        let Some((path, path_at)) = fields
            .get("path")
            .and_then(|value| static_path(source, value))
        else {
            continue;
        };
        if path.starts_with('/') || path.contains(['\\', '?', '#', '\0']) || path == "**" {
            continue;
        }
        let component = fields
            .get("component")
            .and_then(|value| identifier_ref(value))
            .map(|(name, at)| ComponentRef {
                name,
                at,
                kind: "angular_route_component",
            });
        let children = if let Some(children) = fields.get("children") {
            let Expression::ArrayExpression(children) = children else {
                continue;
            };
            parse_array(source, children, depth + 1, budget, context)?
        } else {
            Vec::new()
        };
        result.push(RouteEntry {
            path,
            path_at,
            component,
            children,
        });
        if result.len() > MAX_ROUTES {
            return Err(LIMIT);
        }
    }
    Ok(result)
}

fn imported_route_type(ty: &TSType<'_>, imports: &RouterImports) -> bool {
    match ty {
        TSType::TSTypeReference(reference) => match &reference.type_name {
            TSTypeName::IdentifierReference(identifier) => {
                imports.route_types.contains(identifier.name.as_str())
            }
            _ => false,
        },
        TSType::TSArrayType(array) => imported_route_type(&array.element_type, imports),
        _ => false,
    }
}

fn route_table_binding<'a>(
    pattern: &BindingPattern<'a>,
    imports: &RouterImports,
) -> Option<&'a str> {
    let BindingPatternKind::BindingIdentifier(identifier) = &pattern.kind else {
        return None;
    };
    let annotation = pattern.type_annotation.as_ref()?;
    imported_route_type(&annotation.type_annotation, imports).then_some(identifier.name.as_str())
}

fn variable_symbol<'a>(
    semantic: &'a Semantic<'a>,
    declarator: &VariableDeclarator<'a>,
    name: &str,
) -> Option<SymbolId> {
    semantic.scoping().symbol_ids().find(|symbol| {
        semantic.scoping().symbol_name(*symbol) == name
            && matches!(
                semantic.symbol_declaration(*symbol).kind(),
                AstKind::VariableDeclarator(variable) if variable.span == declarator.span
            )
    })
}

fn is_unescaped_single_use_table(
    semantic: &Semantic<'_>,
    symbol: SymbolId,
    registration_span: oxc_span::Span,
) -> bool {
    if semantic.scoping().symbol_is_mutated(symbol)
        || semantic
            .nodes()
            .ancestors(semantic.symbol_declaration(symbol).id())
            .any(|node| {
                matches!(
                    node.kind(),
                    AstKind::ExportNamedDeclaration(_) | AstKind::ExportDefaultDeclaration(_)
                )
            })
    {
        return false;
    }
    let mut references = semantic.symbol_references(symbol);
    let Some(reference) = references.next() else {
        return false;
    };
    references.next().is_none()
        && semantic.nodes().get_node(reference.node_id()).kind().span() == registration_span
}

fn collect_registration<'a>(
    expression: &'a Expression<'a>,
    imports: &RouterImports,
    arrays: &mut Vec<(&'a ArrayExpression<'a>, Registration)>,
    names: &mut BTreeMap<String, Vec<(Registration, oxc_span::Span)>>,
) {
    let Some(argument) = registered_call(expression, imports) else {
        return;
    };
    match argument {
        RouteArgument::Array(array, registration) => arrays.push((array, registration)),
        RouteArgument::Identifier(name, registration, span) => {
            names.entry(name).or_default().push((registration, span));
        }
    }
}

fn collect_nested_registrations<'a>(
    expression: &'a Expression<'a>,
    imports: &RouterImports,
    arrays: &mut Vec<(&'a ArrayExpression<'a>, Registration)>,
    names: &mut BTreeMap<String, Vec<(Registration, oxc_span::Span)>>,
    context: &OperationContext,
    depth: usize,
    visited: &mut usize,
) -> Result<(), Failure> {
    context.check()?;
    *visited += 1;
    if depth > MAX_DEPTH || *visited > MAX_SCAN_ITEMS {
        return Ok(());
    }
    collect_registration(expression, imports, arrays, names);
    match expression {
        Expression::CallExpression(call) => {
            for argument in &call.arguments {
                if let Argument::SpreadElement(_) = argument {
                    continue;
                }
                if let Some(expression) = argument.as_expression() {
                    collect_nested_registrations(
                        expression,
                        imports,
                        arrays,
                        names,
                        context,
                        depth + 1,
                        visited,
                    )?;
                }
            }
        }
        Expression::ArrayExpression(array) => {
            for element in &array.elements {
                if let Some(expression) = element.as_expression() {
                    collect_nested_registrations(
                        expression,
                        imports,
                        arrays,
                        names,
                        context,
                        depth + 1,
                        visited,
                    )?;
                }
            }
        }
        Expression::ObjectExpression(object) => {
            for property in &object.properties {
                if let ObjectPropertyKind::ObjectProperty(property) = property {
                    collect_nested_registrations(
                        &property.value,
                        imports,
                        arrays,
                        names,
                        context,
                        depth + 1,
                        visited,
                    )?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn classes<'a>(program: &'a oxc_ast::ast::Program<'a>) -> Vec<&'a Class<'a>> {
    program
        .body
        .iter()
        .filter_map(|statement| match statement {
            Statement::ClassDeclaration(class) => Some(class.as_ref()),
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(oxc_ast::ast::Declaration::ClassDeclaration(class)) => Some(class.as_ref()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

pub(super) fn registered_routes(
    source: &str,
    path: &str,
    context: &OperationContext,
) -> Result<Vec<(Vec<RouteEntry>, Registration)>, Failure> {
    if !source.contains("@angular/router") || source.len() > 1024 * 1024 {
        return Ok(Vec::new());
    }
    let source_type = oxc_span::SourceType::from_path(path.to_ascii_lowercase())
        .map_err(|_| ("SYNC_LANGUAGE", "Unsupported Angular route source"))?;
    let allocator = oxc_allocator::Allocator::default();
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();
    context.check()?;
    if !parsed.errors.is_empty() {
        return Ok(Vec::new());
    }
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&parsed.program);
    context.check()?;
    if !semantic.errors.is_empty() {
        return Ok(Vec::new());
    }
    let imports = router_imports(&parsed.program);
    if imports.value_bindings.is_empty() && imports.namespaces.is_empty() {
        return Ok(Vec::new());
    }
    let mut arrays = Vec::new();
    let mut registered_names = BTreeMap::<String, Vec<(Registration, oxc_span::Span)>>::new();
    let mut declarations = Vec::new();
    let mut visited = 0usize;
    for statement in &parsed.program.body {
        context.check()?;
        match statement {
            Statement::VariableDeclaration(declaration) => {
                for declarator in &declaration.declarations {
                    let Some(initializer) = &declarator.init else {
                        continue;
                    };
                    declarations.push((
                        declarator,
                        declaration.kind == VariableDeclarationKind::Const,
                    ));
                    collect_nested_registrations(
                        initializer,
                        &imports,
                        &mut arrays,
                        &mut registered_names,
                        context,
                        0,
                        &mut visited,
                    )?;
                }
            }
            Statement::ExportNamedDeclaration(export) => {
                if let Some(oxc_ast::ast::Declaration::VariableDeclaration(declaration)) =
                    &export.declaration
                {
                    for declarator in &declaration.declarations {
                        let Some(initializer) = &declarator.init else {
                            continue;
                        };
                        declarations.push((
                            declarator,
                            declaration.kind == VariableDeclarationKind::Const,
                        ));
                        collect_nested_registrations(
                            initializer,
                            &imports,
                            &mut arrays,
                            &mut registered_names,
                            context,
                            0,
                            &mut visited,
                        )?;
                    }
                }
            }
            Statement::ExpressionStatement(expression) => collect_nested_registrations(
                &expression.expression,
                &imports,
                &mut arrays,
                &mut registered_names,
                context,
                0,
                &mut visited,
            )?,
            _ => {}
        }
    }
    for class in classes(&parsed.program) {
        for decorator in &class.decorators {
            collect_nested_registrations(
                &decorator.expression,
                &imports,
                &mut arrays,
                &mut registered_names,
                context,
                0,
                &mut visited,
            )?;
        }
    }
    let mut budget = 0usize;
    let mut output = Vec::new();
    for (array, registration) in arrays {
        context.check()?;
        output.push((
            parse_array(source, array, 0, &mut budget, context)?,
            registration,
        ));
    }
    for (declarator, is_const) in declarations {
        if !is_const {
            continue;
        }
        let Some(name) = route_table_binding(&declarator.id, &imports) else {
            continue;
        };
        let Some(registrations) = registered_names.get(name).filter(|items| items.len() == 1)
        else {
            continue;
        };
        let Some(symbol) = variable_symbol(&semantic.semantic, declarator, name) else {
            continue;
        };
        if !is_unescaped_single_use_table(&semantic.semantic, symbol, registrations[0].1) {
            continue;
        }
        if let Some(initializer) = &declarator.init
            && let Expression::ArrayExpression(array) = initializer
        {
            output.push((
                parse_array(source, array, 0, &mut budget, context)?,
                registrations[0].0,
            ));
        }
    }
    Ok(output)
}
