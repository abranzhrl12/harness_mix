use super::{
    INVALID, Implementation, LIMIT, MAX_BYTES, MAX_FACTS, declared_symbol, has_direct_eval,
};
use crate::adapters::index_extract::framework::Positions;
use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast::*};
use oxc_parser::Parser;
use oxc_semantic::{Semantic, SemanticBuilder};
use oxc_span::{GetSpan, SourceType, Span};
use oxc_syntax::symbol::SymbolId;
use std::collections::{BTreeMap, BTreeSet};

/// Prove a direct returned-object property names one local function value.
pub(in crate::adapters) fn returned_method(
    source: &str,
    path: &str,
    function_name: &str,
    property: &str,
    context: &OperationContext,
) -> Result<Option<Implementation>, Failure> {
    context.check()?;
    if source.len() > MAX_BYTES {
        return Err(LIMIT);
    }
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).map_err(|_| INVALID)?;
    let parsed = Parser::new(&allocator, source, source_type).parse();
    context.check()?;
    if parsed.diagnostics.has_errors() {
        return Err(INVALID);
    }
    let built = SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(true)
        .build(&parsed.program);
    context.check()?;
    if built.diagnostics.has_errors() || built.semantic.scoping().symbols_len() > MAX_FACTS {
        return Err(INVALID);
    }
    let semantic = built.semantic;
    if has_direct_eval(&semantic) {
        return Ok(None);
    }
    let positions = Positions::new(source);
    let Some(symbol) = semantic
        .scoping()
        .get_binding(semantic.scoping().root_scope_id(), function_name.into())
    else {
        return Ok(None);
    };
    let declaration = semantic.symbol_declaration(symbol).kind();
    let (body, expression_return, async_, generator) = match declaration {
        AstKind::Function(function) if function.body.is_some() => (
            function.body.as_ref().unwrap().statements.as_slice(),
            false,
            function.r#async,
            function.generator,
        ),
        AstKind::VariableDeclarator(variable)
            if semantic.scoping().symbol_flags(symbol).is_const_variable() =>
        {
            match variable.init.as_ref() {
                Some(Expression::ArrowFunctionExpression(arrow)) => {
                    let (statements, is_expression) = match &arrow.body {
                        oxc_ast::ast::ArrowFunctionBody::FunctionBody(body) => {
                            (body.statements.as_slice(), false)
                        }
                        _ => (&[][..], true),
                    };
                    (statements, is_expression, arrow.r#async, false)
                }
                Some(Expression::FunctionExpression(function)) if function.body.is_some() => (
                    function.body.as_ref().unwrap().statements.as_slice(),
                    false,
                    function.r#async,
                    function.generator,
                ),
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };
    if async_
        || generator
        || semantic.scoping().symbol_is_mutated(symbol)
        || !semantic.scoping().symbol_redeclarations(symbol).is_empty()
    {
        return Ok(None);
    }

    let mut local_functions = BTreeMap::<String, (usize, SymbolId)>::new();
    let mut returned = None;
    if expression_return {
        let [Statement::ExpressionStatement(statement)] = body else {
            return Ok(None);
        };
        let Expression::ObjectExpression(object) = &statement.expression else {
            return Ok(None);
        };
        returned = Some(object);
    }
    let mut saw_return = expression_return;
    for statement in body.iter().filter(|_| !expression_return) {
        context.check()?;
        if saw_return {
            return Ok(None);
        }
        match statement {
            Statement::FunctionDeclaration(function) if function.body.is_some() => {
                let Some(id) = &function.id else {
                    return Ok(None);
                };
                if function.generator {
                    return Ok(None);
                }
                let Some(symbol) = function_symbol(&semantic, function.span, id.name.as_str())
                else {
                    return Ok(None);
                };
                if semantic.scoping().symbol_is_mutated(symbol)
                    || !semantic.scoping().symbol_redeclarations(symbol).is_empty()
                {
                    return Ok(None);
                }
                local_functions.insert(id.name.to_string(), (function.span.start as usize, symbol));
            }
            Statement::VariableDeclaration(declaration) => {
                for variable in &declaration.declarations {
                    if declaration.kind != VariableDeclarationKind::Const {
                        return Ok(None);
                    }
                    let BindingPattern::BindingIdentifier(id) = &variable.id else {
                        return Ok(None);
                    };
                    let Some(symbol) = declared_symbol(&semantic, variable.span, id.name.as_str())
                    else {
                        return Ok(None);
                    };
                    if semantic.scoping().symbol_is_mutated(symbol)
                        || !semantic.scoping().symbol_redeclarations(symbol).is_empty()
                    {
                        return Ok(None);
                    }
                    match variable.init.as_ref() {
                        Some(Expression::ArrowFunctionExpression(arrow)) => {
                            local_functions
                                .insert(id.name.to_string(), (arrow.span.start as usize, symbol));
                        }
                        Some(Expression::FunctionExpression(function))
                            if !function.generator && function.body.is_some() =>
                        {
                            local_functions.insert(
                                id.name.to_string(),
                                (function.span.start as usize, symbol),
                            );
                        }
                        Some(_) => {}
                        None => return Ok(None),
                    }
                }
            }
            Statement::ReturnStatement(statement) => {
                if returned.is_some() {
                    return Ok(None);
                }
                let Some(Expression::ObjectExpression(object)) = statement.argument.as_ref() else {
                    return Ok(None);
                };
                returned = Some(object);
                saw_return = true;
            }
            _ => return Ok(None),
        }
    }
    let Some(object) = returned else {
        return Ok(None);
    };
    let mut found = None;
    let mut keys = BTreeSet::new();
    for item in &object.properties {
        context.check()?;
        let ObjectPropertyKind::ObjectProperty(item) = item else {
            return Ok(None);
        };
        if item.computed || item.method || item.kind != PropertyKind::Init {
            return Ok(None);
        }
        let key = match &item.key {
            PropertyKey::StaticIdentifier(id) => id.name.to_string(),
            PropertyKey::StringLiteral(value) => value.value.display().to_string(),
            _ => return Ok(None),
        };
        if !keys.insert(key.clone()) {
            return Ok(None);
        }
        if key != property {
            continue;
        }
        let Expression::Identifier(identifier) = &item.value else {
            return Ok(None);
        };
        let implementation = identifier.name.to_string();
        let Some(&(offset, expected_symbol)) = local_functions.get(&implementation) else {
            return Ok(None);
        };
        let Some(actual_symbol) = reference_symbol(&semantic, identifier.span) else {
            return Ok(None);
        };
        if actual_symbol != expected_symbol
            || semantic.scoping().symbol_is_mutated(actual_symbol)
            || !semantic
                .scoping()
                .symbol_redeclarations(actual_symbol)
                .is_empty()
        {
            return Ok(None);
        }
        let (line, col) = positions.position(offset)?;
        let line = i64::try_from(line).map_err(|_| LIMIT)?;
        let col = i64::try_from(col).map_err(|_| LIMIT)?;
        if found
            .replace(Implementation {
                name: implementation,
                line,
                col,
            })
            .is_some()
        {
            return Ok(None);
        }
    }
    Ok(found)
}

fn function_symbol<'a>(semantic: &'a Semantic<'a>, span: Span, name: &str) -> Option<SymbolId> {
    semantic.scoping().symbol_ids().find(|symbol| {
        semantic.scoping().symbol_name(*symbol) == name
            && matches!(semantic.symbol_declaration(*symbol).kind(), AstKind::Function(function) if function.span == span)
    })
}

fn reference_symbol<'a>(semantic: &'a Semantic<'a>, span: Span) -> Option<SymbolId> {
    semantic.scoping().symbol_ids().find(|symbol| {
        semantic
            .symbol_references(*symbol)
            .any(|reference| semantic.nodes().kind(reference.node_id()).span() == span)
    })
}
