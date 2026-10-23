//! Source-coordinate proofs and import resolution for Vue route components.
use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Expression, ImportDeclarationSpecifier, ModuleExportName, Statement,
};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use serde_json::json;

const LIMIT: Failure = (
    "EXTRACTION_LIMIT",
    "Vue route source exceeds configured limit",
);

fn imported_name(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(id) => id.name.to_string(),
        ModuleExportName::IdentifierReference(id) => id.name.to_string(),
        ModuleExportName::StringLiteral(lit) => lit.value.display().to_string(),
    }
}

/// Re-derive the exact route reference and stable source node before resolving it.
pub(in crate::adapters) fn proves_reference(
    path: &str,
    source: &str,
    from_node_id: &str,
    name: &str,
    kind: &str,
    line: i64,
    column: i64,
    context: &OperationContext,
) -> Result<bool, Failure> {
    let mut graph = json!({"nodes":[],"edges":[],"unresolvedReferences":[]});
    super::enrich(path, source, &mut graph, context)?;
    Ok(graph["unresolvedReferences"]
        .as_array()
        .is_some_and(|references| {
            references.iter().any(|reference| {
                reference["referenceName"] == name
                    && reference["fromNodeId"] == from_node_id
                    && reference["referenceKind"] == kind
                    && reference["line"] == line
                    && reference["column"] == column
                    && reference["filePath"] == path
            })
        }))
}

/// Resolve named runtime import: `import Home from './Home.vue'` or `import { Home } from './Home'`
pub(in crate::adapters) fn named_runtime_import(
    path: &str,
    source: &str,
    local_name: &str,
    context: &OperationContext,
) -> Result<Option<(String, String)>, Failure> {
    if source.len() > 1024 * 1024 {
        return Err(LIMIT);
    }
    let source_type = SourceType::from_path(path.to_ascii_lowercase())
        .map_err(|_| ("SYNC_LANGUAGE", "Unsupported Vue route source"))?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    context.check()?;
    if parsed.diagnostics.has_errors() {
        return Ok(None);
    }
    let semantic = SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(true)
        .build(&parsed.program);
    context.check()?;
    if semantic.diagnostics.has_errors() {
        return Ok(None);
    }
    let mut bindings = Vec::new();
    for statement in &parsed.program.body {
        context.check()?;
        let Statement::ImportDeclaration(declaration) = statement else {
            continue;
        };
        if declaration.import_kind != oxc_ast::ast::ImportOrExportKind::Value {
            continue;
        }
        let Some(specifiers) = &declaration.specifiers else {
            continue;
        };
        for specifier in specifiers {
            match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(specifier)
                    if specifier.import_kind == oxc_ast::ast::ImportOrExportKind::Value
                        && specifier.local.name.as_str() == local_name =>
                {
                    bindings.push((
                        declaration.source.value.display().to_string(),
                        imported_name(&specifier.imported),
                    ));
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier)
                    if specifier.local.name.as_str() == local_name =>
                {
                    bindings.push((
                        declaration.source.value.display().to_string(),
                        "default".to_string(),
                    ));
                }
                _ => {}
            }
        }
    }
    Ok((bindings.len() == 1).then(|| bindings.remove(0)))
}

/// Return explicit default imports; callers match their local names to tags.
pub(in crate::adapters) fn component_runtime_imports(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Vec<(String, String)>, Failure> {
    Ok(runtime_imports(path, source, context)?
        .into_iter()
        .filter(|(_, _, imported)| imported == "default")
        .map(|(local, module, _)| (local, module))
        .collect::<Vec<_>>())
}

pub(in crate::adapters) fn component_key(name: &str) -> String {
    name.chars()
        .filter(|character| *character != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn runtime_imports(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Vec<(String, String, String)>, Failure> {
    if source.len() > 1024 * 1024 {
        return Err(LIMIT);
    }
    let source_type = SourceType::from_path(path.to_ascii_lowercase())
        .map_err(|_| ("SYNC_LANGUAGE", "Unsupported Vue route source"))?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    context.check()?;
    if parsed.diagnostics.has_errors() {
        return Ok(Vec::new());
    }
    let semantic = SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(true)
        .build(&parsed.program);
    context.check()?;
    if semantic.diagnostics.has_errors() {
        return Ok(Vec::new());
    }
    let mut bindings = Vec::new();
    for statement in &parsed.program.body {
        context.check()?;
        let Statement::ImportDeclaration(declaration) = statement else {
            continue;
        };
        if declaration.import_kind != oxc_ast::ast::ImportOrExportKind::Value {
            continue;
        }
        let Some(specifiers) = &declaration.specifiers else {
            continue;
        };
        for specifier in specifiers {
            match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(specifier)
                    if specifier.import_kind == oxc_ast::ast::ImportOrExportKind::Value =>
                {
                    bindings.push((
                        specifier.local.name.to_string(),
                        declaration.source.value.display().to_string(),
                        imported_name(&specifier.imported),
                    ));
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                    bindings.push((
                        specifier.local.name.to_string(),
                        declaration.source.value.display().to_string(),
                        "default".to_string(),
                    ));
                }
                _ => {}
            }
        }
    }
    Ok(bindings)
}

/// Helper to check if an expression is a dynamic import call `import('...')`
pub(super) fn is_dynamic_import_call<'a>(expr: &'a Expression<'a>) -> Option<&'a str> {
    match expr {
        Expression::ImportExpression(import_expr) => {
            if let Expression::StringLiteral(lit) = &import_expr.source {
                lit.value.as_str()
            } else {
                None
            }
        }
        Expression::ArrowFunctionExpression(arrow) => {
            if let Some(body_expr) = arrow.get_expression() {
                is_dynamic_import_call(body_expr)
            } else if let oxc_ast::ast::ArrowFunctionBody::FunctionBody(body) = &arrow.body {
                for stmt in &body.statements {
                    if let Statement::ReturnStatement(ret) = stmt {
                        if let Some(arg) = &ret.argument {
                            if let Expression::ImportExpression(import_expr) = arg {
                                if let Expression::StringLiteral(lit) = &import_expr.source {
                                    return lit.value.as_str();
                                }
                            }
                        }
                    }
                }
                None
            } else {
                None
            }
        }
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                for stmt in &body.statements {
                    if let Statement::ReturnStatement(ret) = stmt {
                        if let Some(arg) = &ret.argument {
                            if let Expression::ImportExpression(import_expr) = arg {
                                if let Expression::StringLiteral(lit) = &import_expr.source {
                                    return lit.value.as_str();
                                }
                            }
                        }
                    }
                }
                None
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Resolve dynamic runtime import directly from component specifier: `() => import('./Home.vue')`
#[allow(dead_code)]
pub(in crate::adapters) fn dynamic_runtime_import(
    path: &str,
    source: &str,
    target_specifier: &str,
    context: &OperationContext,
) -> Result<Option<(String, String)>, Failure> {
    if source.len() > 1024 * 1024 {
        return Err(LIMIT);
    }
    let source_type = SourceType::from_path(path.to_ascii_lowercase())
        .map_err(|_| ("SYNC_LANGUAGE", "Unsupported Vue route source"))?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    context.check()?;
    if parsed.diagnostics.has_errors() {
        return Ok(None);
    }
    Ok(Some((target_specifier.to_string(), "default".to_string())))
}
