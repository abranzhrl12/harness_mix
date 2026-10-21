//! Oxc lexical binding analysis only. CodeGraph still supplies nodes and IDs.
use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast::Expression};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_syntax::module_record::{ExportExportName, ExportImportName, ImportImportName};
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::adapters) mod vue;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::adapters) enum RuntimeKind {
    Function,
    Class,
}

const INVALID: Failure = (
    "SYNC_SEMANTIC",
    "Cannot establish unambiguous JavaScript/TypeScript import bindings",
);
const LIMIT: Failure = (
    "SYNC_LIMIT",
    "Native semantic analysis exceeded its bounded graph budget",
);

#[derive(Default)]
pub(super) struct Calls {
    sites: BTreeSet<(String, i64, i64)>,
    pub(super) exports: BTreeMap<String, Export>,
    // Kept separate: legacy JS/TS resolution retains its narrower policy.
    pub(super) runtime_exports: BTreeMap<String, Export>,
    pub(super) runtime_kinds: BTreeMap<String, RuntimeKind>,
    pub(super) stars: Vec<String>,
    pub(super) reserved: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Export {
    Local(String),
    Remote { module: String, name: String },
    Unsupported,
}
impl Calls {
    pub(super) fn contains(&self, name: &str, line: i64, col: i64) -> bool {
        self.sites.contains(&(name.to_owned(), line, col))
    }
}

pub(super) fn analyze(
    source: &str,
    path: &str,
    context: &OperationContext,
) -> Result<Calls, Failure> {
    context.check()?;
    if source.len() > super::index_sync::MAX_SOURCE_BYTES {
        return Err(LIMIT);
    }
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path.to_ascii_lowercase()).map_err(|_| INVALID)?;
    let parsed = Parser::new(&allocator, source, source_type).parse();
    context.check()?;
    if !parsed.errors.is_empty() {
        return Err((
            "SYNC_SEMANTIC",
            "JavaScript/TypeScript parser rejected the source; inspect syntax diagnostics for this file",
        ));
    }
    let built = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&parsed.program);
    context.check()?;
    if !built.errors.is_empty() {
        return Err((
            "SYNC_SEMANTIC",
            "JavaScript/TypeScript lexical binding analysis rejected the source; inspect duplicate declarations or invalid scope bindings",
        ));
    }
    let semantic = built.semantic;
    if semantic.scoping().symbols_len() > 20_000 {
        return Err(LIMIT);
    }
    let mut offsets = Vec::new();
    let mut reserved = 0usize;
    for symbol in semantic.scoping().symbol_ids() {
        context.check()?;
        // TypeScript may record conflicts without emitting semantic errors.
        if !semantic.scoping().symbol_redeclarations(symbol).is_empty() {
            return Err((
                "SYNC_SEMANTIC",
                "JavaScript/TypeScript binding has multiple declarations; imported/exported bindings cannot be resolved unambiguously",
            ));
        }
        if semantic.scoping().symbol_flags(symbol).is_type_import()
            || !matches!(
                semantic.symbol_declaration(symbol).kind(),
                AstKind::ImportSpecifier(_)
            )
        {
            continue;
        }
        let name = semantic.scoping().symbol_name(symbol);
        for reference in semantic.symbol_references(symbol) {
            context.check()?;
            let AstKind::CallExpression(call) = semantic.nodes().parent_kind(reference.node_id())
            else {
                continue;
            };
            // Do not treat callbacks, property receivers, casts or wrappers as
            // bare calls. Extending their edge semantics is a separate change.
            let Expression::Identifier(identifier) = &call.callee else {
                continue;
            };
            if identifier.name.as_str() != name || call.span.start != identifier.span.start {
                continue;
            }
            reserved += name.len() + 32;
            if offsets.len() >= 20_000 || reserved > 16 * 1024 * 1024 {
                return Err(LIMIT);
            }
            offsets.push((call.span.start as usize, name.to_owned()));
        }
    }
    offsets.sort_unstable();
    let mut result = Calls {
        sites: BTreeSet::new(),
        exports: BTreeMap::new(),
        runtime_exports: BTreeMap::new(),
        runtime_kinds: BTreeMap::new(),
        stars: Vec::new(),
        reserved,
    };
    // Oxc supplies the ESM entries; retain owned, bounded data after its arena
    // is dropped. Only implemented function declarations become graph targets.
    let record = &parsed.module_record;
    for entry in record
        .local_export_entries
        .iter()
        .chain(&record.indirect_export_entries)
    {
        context.check()?;
        // Type-only exports are erased, not runtime shadows of export-star.
        if entry.is_type {
            continue;
        }
        let ExportExportName::Name(exported) = &entry.export_name else {
            continue;
        };
        let name = exported.name.as_str();
        let mut target = Export::Unsupported;
        let mut runtime_target = None;
        if !entry.is_type && name != "default" {
            if let (Some(module), ExportImportName::Name(imported)) =
                (&entry.module_request, &entry.import_name)
            {
                if imported.name != "default" {
                    target = Export::Remote {
                        module: module.name.to_string(),
                        name: imported.name.to_string(),
                    };
                }
            } else if let Some(local) = entry.local_name.name() {
                let imported = record
                    .import_entries
                    .iter()
                    .find(|i| i.local_name.name == local);
                if let Some(imported) = imported {
                    if !imported.is_type
                        && let ImportImportName::Name(original) = &imported.import_name
                        && original.name != "default"
                    {
                        target = Export::Remote {
                            module: imported.module_request.name.to_string(),
                            name: original.name.to_string(),
                        };
                    }
                } else if let Some(symbol) = semantic
                    .scoping()
                    .get_binding(semantic.scoping().root_scope_id(), local.as_str())
                {
                    let declaration = semantic.symbol_declaration(symbol).kind();
                    let kind = match declaration {
                        AstKind::Function(function) if function.body.is_some() => {
                            target = Export::Local(local.to_string());
                            Some(RuntimeKind::Function)
                        }
                        AstKind::Class(class) if !class.declare => Some(RuntimeKind::Class),
                        AstKind::VariableDeclarator(variable)
                            if semantic.scoping().symbol_flags(symbol).is_const_variable()
                                && matches!(
                                    &variable.id.kind,
                                    oxc_ast::ast::BindingPatternKind::BindingIdentifier(_)
                                )
                                && matches!(
                                    &variable.init,
                                    Some(Expression::ArrowFunctionExpression(_))
                                        | Some(Expression::FunctionExpression(_))
                                ) =>
                        {
                            Some(RuntimeKind::Function)
                        }
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        runtime_target = Some(Export::Local(local.to_string()));
                        result.runtime_kinds.insert(local.to_string(), kind);
                    }
                }
            }
        }
        let bytes = name.len()
            + match &target {
                Export::Local(local) => local.len(),
                Export::Remote { module, name } => module.len() + name.len(),
                Export::Unsupported => 0,
            };
        let runtime_target = runtime_target.unwrap_or_else(|| target.clone());
        // Account for both owned export views plus local-kind keys.
        result.reserved += (bytes + 64).saturating_mul(3);
        if result.exports.len() >= 20_000 || result.reserved > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        if result.exports.insert(name.into(), target).is_some() {
            return Err(INVALID);
        }
        if result
            .runtime_exports
            .insert(name.into(), runtime_target)
            .is_some()
        {
            return Err(INVALID);
        }
    }
    for entry in &record.star_export_entries {
        context.check()?;
        if entry.is_type {
            continue;
        }
        let Some(module) = &entry.module_request else {
            return Err(INVALID);
        };
        result.reserved += module.name.len() + 32;
        if result.stars.len() >= 256 || result.reserved > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        result.stars.push(module.name.to_string());
    }
    // One monotonic source scan: avoid rescanning a long line for every call.
    // CodeGraph uses 1-based LF lines and 0-based UTF-16 columns, not byte offsets.
    let mut cursor = 0;
    let mut line = 1i64;
    let mut col = 0i64;
    for (offset, name) in offsets {
        context.check()?;
        let prefix = source.get(cursor..offset).ok_or(INVALID)?;
        for ch in prefix.chars() {
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += ch.len_utf16() as i64;
            }
        }
        cursor = offset;
        result.sites.insert((name, line, col));
    }
    Ok(result)
}

#[cfg(test)]
#[path = "index_semantic/tests.rs"]
mod tests;
