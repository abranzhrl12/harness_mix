//! Static recognition of Angular HttpClient calls backed by an imported HttpClient token.
use oxc_ast::{
    AstKind,
    ast::{
        Argument, Class, ClassElement, Expression, FormalParameter, MethodDefinitionKind,
        PropertyKey, TSType, TSTypeName,
    },
};
use oxc_semantic::Semantic;
use oxc_span::GetSpan;
use std::collections::BTreeSet;

fn module_export<'a>(name: &'a oxc_ast::ast::ModuleExportName<'a>) -> &'a str {
    match name {
        oxc_ast::ast::ModuleExportName::IdentifierName(id) => id.name.as_str(),
        oxc_ast::ast::ModuleExportName::IdentifierReference(id) => id.name.as_str(),
        oxc_ast::ast::ModuleExportName::StringLiteral(value) => value.value.as_str().unwrap_or(""),
    }
}
fn live_import(
    semantic: &Semantic<'_>,
    name: &str,
    start: u32,
    end: u32,
    imported: &str,
    module: &str,
) -> bool {
    let Some(symbol) = semantic
        .scoping()
        .get_binding(semantic.scoping().root_scope_id(), name.into())
    else {
        return false;
    };
    if !semantic.scoping().symbol_redeclarations(symbol).is_empty()
        || semantic.scoping().symbol_flags(symbol).is_type_import()
    {
        return false;
    }
    let declaration = semantic.symbol_declaration(symbol);
    let AstKind::ImportSpecifier(specifier) = declaration.kind() else {
        return false;
    };
    specifier.import_kind == oxc_ast::ast::ImportOrExportKind::Value
        && module_export(&specifier.imported) == imported
        && semantic.symbol_references(symbol).any(|reference| {
            let span = semantic.nodes().get_node(reference.node_id()).kind().span();
            span.start == start && span.end == end
        })
        && semantic.nodes().ancestors(declaration.id()).any(|node| {
            matches!(node.kind(), AstKind::ImportDeclaration(import) if import.source.value == module)
        })
}
fn is_http_client(
    semantic: &Semantic<'_>,
    identifier: &oxc_ast::ast::IdentifierReference<'_>,
) -> bool {
    live_import(
        semantic,
        identifier.name.as_str(),
        identifier.span.start,
        identifier.span.end,
        "HttpClient",
        "@angular/common/http",
    )
}
fn is_http_type(
    semantic: &Semantic<'_>,
    annotation: Option<&oxc_ast::ast::TSTypeAnnotation<'_>>,
) -> bool {
    let Some(annotation) = annotation else {
        return false;
    };
    let TSType::TSTypeReference(reference) = &annotation.type_annotation else {
        return false;
    };
    let TSTypeName::IdentifierReference(identifier) = &reference.type_name else {
        return false;
    };
    is_http_client(semantic, identifier)
}
fn static_key<'a>(key: &'a PropertyKey<'a>) -> Option<&'a str> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.as_str()),
        PropertyKey::PrivateIdentifier(id) => Some(id.name.as_str()),
        _ => None,
    }
}
fn parameter_name<'a>(parameter: &'a FormalParameter<'a>) -> Option<&'a str> {
    match &parameter.pattern {
        oxc_ast::ast::BindingPattern::BindingIdentifier(id) => Some(id.name.as_str()),
        _ => None,
    }
}
fn injected_http(field: &oxc_ast::ast::PropertyDefinition<'_>, semantic: &Semantic<'_>) -> bool {
    let Some(Expression::CallExpression(call)) = &field.value else {
        return false;
    };
    let Expression::Identifier(callee) = &call.callee else {
        return false;
    };
    if !live_import(
        semantic,
        callee.name.as_str(),
        callee.span.start,
        callee.span.end,
        "inject",
        "@angular/core",
    ) {
        return false;
    }
    matches!(call.arguments.first(), Some(Argument::Identifier(token)) if is_http_client(semantic, token))
}
fn class_clients(class: &Class<'_>, semantic: &Semantic<'_>) -> BTreeSet<String> {
    let mut properties = BTreeSet::new();
    for element in &class.body.body {
        match element {
            ClassElement::PropertyDefinition(field)
                if !field.r#static && injected_http(field, semantic) =>
            {
                if let Some(name) = static_key(&field.key) {
                    properties.insert(name.to_owned());
                }
            }
            ClassElement::MethodDefinition(method)
                if method.kind == MethodDefinitionKind::Constructor =>
            {
                for parameter in &method.value.params.items {
                    if (parameter.accessibility.is_some() || parameter.readonly)
                        && is_http_type(semantic, parameter.type_annotation.as_deref())
                    {
                        if let Some(name) = parameter_name(parameter) {
                            properties.insert(name.to_owned());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    properties
}
struct ClassClients {
    start: u32,
    end: u32,
    properties: BTreeSet<String>,
    allowed_calls: Vec<std::ops::Range<u32>>,
    method_functions: BTreeSet<(u32, u32)>,
}
#[derive(Default)]
pub(super) struct Clients {
    classes: Vec<ClassClients>,
    proven_calls: BTreeSet<u32>,
}
impl Clients {
    pub(super) fn new(program: &oxc_ast::ast::Program<'_>, semantic: &Semantic<'_>) -> Self {
        let mut result = Self::default();
        for statement in &program.body {
            let class = match statement {
                oxc_ast::ast::Statement::ClassDeclaration(class) => Some(class.as_ref()),
                oxc_ast::ast::Statement::ExportDeclaration(export) => match &export.declaration {
                    oxc_ast::ast::Declaration::ClassDeclaration(class) => Some(class.as_ref()),
                    _ => None,
                },
                oxc_ast::ast::Statement::ExportDefaultDeclaration(export) => {
                    match &export.declaration {
                        oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                            Some(class.as_ref())
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(class) = class {
                let mut fact = ClassClients {
                    start: class.span.start,
                    end: class.span.end,
                    properties: class_clients(class, semantic),
                    allowed_calls: Vec::new(),
                    method_functions: BTreeSet::new(),
                };
                for element in &class.body.body {
                    let ClassElement::MethodDefinition(method) = element else {
                        continue;
                    };
                    if method.r#static || method.value.body.is_none() {
                        continue;
                    }
                    fact.allowed_calls.push(
                        method.value.body.as_ref().unwrap().span.start as u32
                            ..method.value.body.as_ref().unwrap().span.end as u32,
                    );
                    fact.method_functions
                        .insert((method.value.span.start, method.value.span.end));
                }
                result.classes.push(fact);
            }
        }
        for node in semantic.nodes().iter() {
            let AstKind::CallExpression(call) = node.kind() else {
                continue;
            };
            let Expression::StaticMemberExpression(member) = &call.callee else {
                continue;
            };
            let Expression::StaticMemberExpression(client) = &member.object else {
                continue;
            };
            let Expression::ThisExpression(_) = &client.object else {
                continue;
            };
            let property = client.property.name.as_str();
            let Some(class) = result
                .classes
                .iter()
                .filter(|class| class.start <= call.span.start && call.span.end <= class.end)
                .min_by_key(|class| class.end - class.start)
            else {
                continue;
            };
            if !class.properties.contains(property)
                || !class
                    .allowed_calls
                    .iter()
                    .any(|range| range.start <= call.span.start && call.span.end <= range.end)
            {
                continue;
            }
            let direct_context = semantic
                .nodes()
                .ancestors(node.id())
                .take_while(|ancestor| {
                    let span = ancestor.kind().span();
                    !(span.start == class.start
                        && span.end == class.end
                        && matches!(ancestor.kind(), AstKind::Class(_)))
                })
                .all(|ancestor| match ancestor.kind() {
                    AstKind::Class(_) => false,
                    AstKind::ArrowFunctionExpression(_) => true,
                    AstKind::Function(function) => class
                        .method_functions
                        .contains(&(function.span.start, function.span.end)),
                    _ => true,
                });
            if direct_context {
                result.proven_calls.insert(call.span.start);
            }
        }
        result
    }
    fn owns(&self, call: &oxc_ast::ast::CallExpression<'_>) -> bool {
        self.proven_calls.contains(&call.span.start)
    }
}
fn literal(source: &str, expression: &Expression<'_>) -> Option<String> {
    let raw = source.get(expression.span().start as usize..expression.span().end as usize)?;
    match expression {
        Expression::StringLiteral(value) => {
            let quote = *raw.as_bytes().first()?;
            if !matches!(quote, b'\'' | b'"') || raw.as_bytes().last().copied() != Some(quote) {
                return None;
            }
            let inner = raw.get(1..raw.len().checked_sub(1)?)?;
            (!inner.contains('\\') && value.value == inner).then(|| inner.to_owned())
        }
        Expression::TemplateLiteral(value)
            if value.expressions.is_empty() && value.quasis.len() == 1 =>
        {
            let inner = raw.get(1..raw.len().checked_sub(1)?)?;
            let cooked = value.quasis[0].value.cooked?.as_str()?;
            (raw.starts_with('\x60')
                && raw.ends_with('\x60')
                && !inner.contains('\\')
                && inner == cooked)
                .then(|| inner.to_owned())
        }
        _ => None,
    }
}
/// Recognize calls only when this.<property> is proven to be an Angular HttpClient.
pub(super) fn request(
    source: &str,
    call: &oxc_ast::ast::CallExpression<'_>,
    clients: &Clients,
) -> Option<(String, String)> {
    let Expression::StaticMemberExpression(member) = &call.callee else {
        return None;
    };
    let Expression::StaticMemberExpression(client) = &member.object else {
        return None;
    };
    let Expression::ThisExpression(_) = &client.object else {
        return None;
    };
    if !clients.owns(call) {
        return None;
    }
    match member.property.name.as_str() {
        "get" => Some((
            literal(source, call.arguments.first()?.as_expression()?)?,
            "GET".to_owned(),
        )),
        "post" => Some((
            literal(source, call.arguments.first()?.as_expression()?)?,
            "POST".to_owned(),
        )),
        "put" => Some((
            literal(source, call.arguments.first()?.as_expression()?)?,
            "PUT".to_owned(),
        )),
        "patch" => Some((
            literal(source, call.arguments.first()?.as_expression()?)?,
            "PATCH".to_owned(),
        )),
        "delete" => Some((
            literal(source, call.arguments.first()?.as_expression()?)?,
            "DELETE".to_owned(),
        )),
        "head" => Some((
            literal(source, call.arguments.first()?.as_expression()?)?,
            "HEAD".to_owned(),
        )),
        "options" => Some((
            literal(source, call.arguments.first()?.as_expression()?)?,
            "OPTIONS".to_owned(),
        )),
        "request" => {
            let method =
                literal(source, call.arguments.first()?.as_expression()?)?.to_ascii_uppercase();
            if !matches!(
                method.as_str(),
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
            ) {
                return None;
            }
            let url = literal(source, call.arguments.get(1)?.as_expression()?)?;
            Some((url, method))
        }
        _ => None,
    }
}
