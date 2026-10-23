//! Oxc-based static Vue route table recognition.
use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ArrayExpression, BindingPattern, CallExpression, Declaration, Expression,
    ObjectPropertyKind, PropertyKey, Statement,
};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use std::collections::BTreeMap;

pub(super) const MAX_ROUTES: usize = 2048;
const MAX_DEPTH: usize = 12;
const MAX_SCAN_ITEMS: usize = 20_000;
const LIMIT: Failure = (
    "EXTRACTION_LIMIT",
    "Vue route extraction limit exceeded",
);

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
            PropertyKey::StringLiteral(key) => key.value.display().to_string(),
            _ => return None,
        };
        if result.insert(key, &property.value).is_some() {
            return None;
        }
    }
    Some(result)
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
        || value.value != raw.get(1..raw.len() - 1)?
    {
        return None;
    }
    Some((value.value.display().to_string(), start))
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
        if fields.contains_key("redirect") {
            continue;
        }
        let Some((path, path_at)) = fields
            .get("path")
            .and_then(|value| static_path(source, value))
        else {
            continue;
        };
        if path.contains(['\\', '?', '#', '\0']) || path == "*" {
            continue;
        }
        let component = if let Some(comp_expr) = fields.get("component") {
            match comp_expr {
                Expression::Identifier(ident) => Some(ComponentRef {
                    name: ident.name.to_string(),
                    at: ident.span.start as usize,
                    kind: "vue_route_component",
                }),
                expr => {
                    if let Some(specifier) = super::bindings::is_dynamic_import_call(expr) {
                        Some(ComponentRef {
                            name: specifier.to_string(),
                            at: expr.span().start as usize,
                            kind: "vue_route_dynamic_component",
                        })
                    } else {
                        None
                    }
                }
            }
        } else {
            None
        };
        let children = if let Some(children_expr) = fields.get("children") {
            if let Expression::ArrayExpression(child_arr) = children_expr {
                parse_array(source, child_arr, depth + 1, budget, context)?
            } else {
                Vec::new()
            }
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

pub(super) fn registered_routes<'a>(
    source: &'a str,
    path: &str,
    context: &OperationContext,
) -> Result<Vec<Vec<RouteEntry>>, Failure> {
    if source.len() > 1024 * 1024 {
        return Err(LIMIT);
    }
    let source_type = SourceType::from_path(path.to_ascii_lowercase())
        .map_err(|_| ("SYNC_LANGUAGE", "Unsupported Vue router source"))?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    context.check()?;
    if parsed.diagnostics.has_errors() {
        return Ok(Vec::new());
    }

    let mut route_arrays = Vec::<&ArrayExpression<'_>>::new();
    let mut candidate_arrays = BTreeMap::<String, &ArrayExpression<'_>>::new();
    let mut create_router_calls = Vec::<&CallExpression<'_>>::new();

    // Scan top-level statements for variables and createRouter calls
    for stmt in &parsed.program.body {
        match stmt {
            Statement::VariableDeclaration(decl) => {
                for declarator in &decl.declarations {
                    if let BindingPattern::BindingIdentifier(ident) = &declarator.id {
                        let var_name = ident.name.as_str();
                        if let Some(init) = &declarator.init {
                            match init {
                                Expression::ArrayExpression(arr) => {
                                    candidate_arrays.insert(var_name.to_string(), arr.as_ref());
                                }
                                Expression::CallExpression(call) => {
                                    if is_create_router_call(call) {
                                        create_router_calls.push(call.as_ref());
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            Statement::ExportDeclaration(export) => {
                if let Declaration::VariableDeclaration(decl) = &export.declaration {
                    for declarator in &decl.declarations {
                        if let BindingPattern::BindingIdentifier(ident) = &declarator.id {
                            let var_name = ident.name.as_str();
                            if let Some(init) = &declarator.init {
                                match init {
                                    Expression::ArrayExpression(arr) => {
                                        candidate_arrays.insert(var_name.to_string(), arr.as_ref());
                                    }
                                    Expression::CallExpression(call) => {
                                        if is_create_router_call(call) {
                                            create_router_calls.push(call.as_ref());
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            Statement::ExpressionStatement(expr_stmt) => {
                if let Expression::CallExpression(call) = &expr_stmt.expression {
                    if is_create_router_call(call) {
                        create_router_calls.push(call.as_ref());
                    }
                }
            }
            _ => {}
        }
    }

    // Check createRouter calls
    for call in create_router_calls {
        if let Some(first_arg) = call.arguments.first() {
            if let Some(Expression::ObjectExpression(obj)) = first_arg.as_expression() {
                if let Some(props) = properties(obj) {
                    if let Some(routes_prop) = props.get("routes") {
                        match routes_prop {
                            Expression::ArrayExpression(arr) => {
                                route_arrays.push(arr.as_ref());
                            }
                            Expression::Identifier(ident) => {
                                if let Some(arr) = candidate_arrays.get(ident.name.as_str()) {
                                    route_arrays.push(*arr);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    // Also: if no createRouter calls found, but there is a variable named `routes` with array expression
    if route_arrays.is_empty() {
        if let Some(arr) = candidate_arrays.get("routes") {
            route_arrays.push(*arr);
        }
    }

    let mut budget = 0usize;
    let mut output = Vec::new();
    for arr in route_arrays {
        context.check()?;
        output.push(parse_array(source, arr, 0, &mut budget, context)?);
    }
    Ok(output)
}

fn is_create_router_call(call: &CallExpression<'_>) -> bool {
    match &call.callee {
        Expression::Identifier(ident) => ident.name.as_str() == "createRouter",
        Expression::StaticMemberExpression(member) => member.property.name.as_str() == "createRouter",
        _ => false,
    }
}
