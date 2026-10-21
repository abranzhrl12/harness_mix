use super::INVALID;
use harness_domain::Failure;
use oxc_ast::{
    AstKind,
    ast::{Program, Statement},
};
use oxc_semantic::Semantic;
use oxc_span::Span;
use oxc_syntax::symbol::SymbolId;
use std::collections::BTreeMap;

pub(super) fn named_imports(
    program: &Program,
    source: &str,
) -> Result<BTreeMap<String, (String, String)>, Failure> {
    let mut imports = BTreeMap::new();
    for statement in &program.body {
        let Statement::ImportDeclaration(declaration) = statement else {
            continue;
        };
        let start = declaration.span.start as usize;
        let end = declaration.span.end as usize;
        let text = source.get(start..end).ok_or(INVALID)?;
        let Some((module, pairs)) = codegraph_native::named_imports(text) else {
            continue;
        };
        for (imported, local) in pairs {
            if imports.insert(local, (module.clone(), imported)).is_some() {
                return Err(INVALID);
            }
        }
    }
    Ok(imports)
}

pub(super) fn declared_symbol<'a>(
    semantic: &'a Semantic<'a>,
    span: Span,
    name: &str,
) -> Option<SymbolId> {
    semantic.scoping().symbol_ids().find(|symbol| {
        semantic.scoping().symbol_name(*symbol) == name
            && matches!(semantic.symbol_declaration(*symbol).kind(), AstKind::VariableDeclarator(v) if v.span == span)
    })
}

pub(super) fn is_exported_binding(semantic: &Semantic<'_>, symbol: SymbolId) -> bool {
    semantic
        .nodes()
        .ancestors(semantic.symbol_declaration(symbol).id())
        .any(|node| {
            matches!(
                node.kind(),
                AstKind::ExportNamedDeclaration(_) | AstKind::ExportDefaultDeclaration(_)
            )
        })
}
