//! Source-coordinate proofs used by the index-store resolver.
use super::parse::imported_name;
use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_ast::ast::{ImportDeclarationSpecifier, Statement};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use serde_json::json;

const LIMIT: Failure = (
    "EXTRACTION_LIMIT",
    "Angular route source exceeds configured limit",
);

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

/// Resolve only a unique current runtime named import; type-only and default imports defer.
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
        .map_err(|_| ("SYNC_LANGUAGE", "Unsupported Angular route source"))?;
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
            if let ImportDeclarationSpecifier::ImportSpecifier(specifier) = specifier
                && specifier.import_kind == oxc_ast::ast::ImportOrExportKind::Value
                && specifier.local.name.as_str() == local_name
            {
                bindings.push((
                    declaration.source.value.display().to_string(),
                    imported_name(&specifier.imported),
                ));
            }
        }
    }
    Ok((bindings.len() == 1).then(|| bindings.remove(0)))
}
