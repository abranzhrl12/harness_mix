//! Oxc lexical binding analysis only. CodeGraph still supplies nodes and IDs.
use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast::Expression};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_syntax::module_record::{ExportExportName, ExportImportName, ImportImportName};
use std::collections::{BTreeMap, BTreeSet};

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
    let source_type = SourceType::from_path(path).map_err(|_| INVALID)?;
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
                    && let AstKind::Function(function) = semantic.symbol_declaration(symbol).kind()
                    && function.body.is_some()
                {
                    target = Export::Local(local.to_string());
                }
            }
        }
        let bytes = name.len()
            + match &target {
                Export::Local(local) => local.len(),
                Export::Remote { module, name } => module.len() + name.len(),
                Export::Unsupported => 0,
            };
        result.reserved += bytes + 64;
        if result.exports.len() >= 20_000 || result.reserved > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        if result.exports.insert(name.into(), target).is_some() {
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
mod tests {
    use super::*;

    #[test]
    fn legacy_datatables_helper_can_be_analyzed() {
        let source = include_str!("../../tests/fixtures/datatables-helper.js");
        let result = analyze(source, "datatables-helper.js", &OperationContext::default());
        assert!(result.is_ok(), "{:?}", result.err());
    }
    #[test]
    fn oxc_type_exports_are_not_runtime_star_shadows() {
        let context = OperationContext::default();
        let typed = analyze("export type * from './types';", "barrel.ts", &context).unwrap();
        assert!(typed.stars.is_empty());
        assert!(typed.exports.is_empty());
        let mixed = analyze(
            "export type {Api as api} from './types'; export * from './leaf';",
            "barrel.ts",
            &context,
        )
        .unwrap();
        assert!(!mixed.exports.contains_key("api"));
        assert_eq!(mixed.stars, ["./leaf"]);
    }
    #[test]
    fn lexical_scopes_keep_import_call_but_not_shadowed_calls() {
        let source = "import {run as execute} from './lib';\nfunction local(execute: () => void) { execute(); }\nexport function caller() { execute(); }";
        let calls = analyze(source, "app.ts", &OperationContext::default()).unwrap();
        assert_eq!(calls.sites.len(), 1);
        assert!(calls.contains("execute", 3, 27));
    }
    #[test]
    fn semantic_positions_match_codegraph_with_unicode_and_crlf() {
        let source = "import {run as execute} from './lib';\r\nexport function caller() { const emoji = '😀'; execute(); }\r\n";
        let context = OperationContext::default();
        let calls = analyze(source, "app.ts", &context).unwrap();
        let graph = super::super::index_extract::extract("app.ts", source, &context).unwrap();
        let reference = graph["unresolvedReferences"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["referenceKind"] == "calls" && r["referenceName"] == "execute")
            .unwrap();
        assert!(calls.contains(
            "execute",
            reference["line"].as_i64().unwrap(),
            reference["column"].as_i64().unwrap()
        ));
        assert_eq!(calls.sites.len(), 1);
    }
    #[test]
    fn callbacks_members_and_type_only_imports_are_not_direct_import_calls() {
        for source in [
            "import {run as execute} from './lib'; export function caller() { register(execute); this.execute(); }",
            "import type {run as execute} from './lib'; export function caller() { execute(); }",
        ] {
            assert!(
                analyze(source, "app.ts", &OperationContext::default())
                    .unwrap()
                    .sites
                    .is_empty()
            );
        }
    }
    #[test]
    fn invalid_conflicting_cancelled_and_oversized_sources_are_rejected() {
        let context = OperationContext::default();
        assert!(analyze("function {", "app.ts", &context).is_err());
        assert!(
            analyze(
                "import {run as execute} from './lib'; const execute = () => {}; execute();",
                "app.ts",
                &context
            )
            .is_err()
        );
        assert_eq!(
            analyze(&" ".repeat(1024 * 1024 + 1), "app.ts", &context)
                .err()
                .unwrap()
                .0,
            "SYNC_LIMIT"
        );
        context.cancel();
        assert_eq!(
            analyze("", "app.ts", &context).err().unwrap().0,
            "CANCELLED"
        );
    }
}
