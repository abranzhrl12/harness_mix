//! Bounded proof for Vue composable return-object method calls.
//!
//! This deliberately understands only const destructuring and const receivers
//! initialized directly by a named imported composable call. The returned value
//! must be one unconditional object literal whose selected property names one
//! local function implementation.
use crate::adapters::parsers::vue::structure::{self, Block};
use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast::*};
use oxc_parser::Parser;
use oxc_semantic::{Semantic, SemanticBuilder};
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::symbol::SymbolId;
use std::collections::BTreeMap;

mod imports;
mod target;
use imports::{declared_symbol, is_exported_binding, named_imports};
pub(in crate::adapters) use target::returned_method;

const INVALID: Failure = (
    "SYNC_SEMANTIC",
    "Cannot establish an unambiguous Vue composable returned-method binding",
);
const LIMIT: Failure = (
    "SYNC_LIMIT",
    "Vue returned-method analysis exceeded its bounded graph budget",
);
const MAX_BYTES: usize = 512 * 1024;
const MAX_FACTS: usize = 20_000;
const MAX_RESERVED: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub(in crate::adapters) struct Facts {
    calls: BTreeMap<(String, i64, i64), (String, String, String, String)>,
    pub(in crate::adapters) reserved: usize,
}

#[derive(Clone, Debug)]
pub(in crate::adapters) struct Implementation {
    pub(in crate::adapters) name: String,
    pub(in crate::adapters) line: i64,
    pub(in crate::adapters) col: i64,
}

impl Facts {
    pub(in crate::adapters) fn binding(
        &self,
        name: &str,
        line: i64,
        col: i64,
    ) -> Option<(String, String, String, String)> {
        self.calls.get(&(name.to_owned(), line, col)).cloned()
    }
}

/// Re-derive a Vue script method call from its script-local lexical bindings.
/// Returned tuple: requested module, exported composable, language, property.
pub(in crate::adapters) fn analyze_calls(
    source: &str,
    path: &str,
    context: &OperationContext,
) -> Result<Facts, Failure> {
    context.check()?;
    if source.len() > MAX_BYTES {
        return Err(LIMIT);
    }
    let digest = harness_domain::hash(source.as_bytes());
    let document = structure::parse(source, &digest, context)?;
    let mut facts = Facts::default();
    for block in document
        .blocks
        .iter()
        .filter(|block| block.kind == "script")
    {
        context.check()?;
        if structure::attr(&block.attrs, "src").is_some() {
            continue;
        }
        let Some(language) = script_language(block) else {
            continue;
        };
        let projection =
            crate::adapters::index_extract::framework::project(source, block.body.clone())?;
        let parser_path = format!("{path}.{}", language_extension(language));
        collect_script_calls(&projection, &parser_path, language, context, &mut facts)?;
    }
    Ok(facts)
}

fn collect_script_calls(
    source: &str,
    path: &str,
    language: &str,
    context: &OperationContext,
    facts: &mut Facts,
) -> Result<(), Failure> {
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
    if has_direct_eval(&built.semantic) {
        return Ok(());
    }
    let positions = crate::adapters::index_extract::framework::Positions::new(source);
    let semantic = built.semantic;
    let imports = named_imports(&parsed.program, source)?;
    let mut initializers = Vec::new();

    for symbol in semantic.scoping().symbol_ids() {
        context.check()?;
        if semantic.scoping().symbol_is_unused(symbol) {
            continue;
        }
        if semantic.scoping().symbol_flags(symbol).is_type_import() {
            continue;
        }
        let AstKind::ImportSpecifier(_) = semantic.symbol_declaration(symbol).kind() else {
            continue;
        };
        let local = semantic.scoping().symbol_name(symbol);
        let Some((module, imported)) = imports.get(local) else {
            continue;
        };
        for reference in semantic.symbol_references(symbol) {
            context.check()?;
            let AstKind::CallExpression(call) = semantic.nodes().parent_kind(reference.node_id())
            else {
                continue;
            };
            let Expression::Identifier(identifier) = &call.callee else {
                continue;
            };
            if identifier.name.as_str() != local || !call.arguments.is_empty() || call.optional {
                continue;
            }
            let Some((variable, variable_kind)) = semantic
                .nodes()
                .ancestors(reference.node_id())
                .find_map(|node| {
                    let AstKind::VariableDeclarator(declarator) = node.kind() else {
                        return None;
                    };
                    let AstKind::VariableDeclaration(declaration) =
                        semantic.nodes().parent_kind(node.id())
                    else {
                        return None;
                    };
                    declarator
                        .init
                        .as_ref()
                        .filter(|init| init.span() == call.span)
                        .map(|_| (declarator, declaration.kind))
                })
            else {
                continue;
            };
            if variable_kind != VariableDeclarationKind::Const {
                continue;
            }
            initializers.push((module.clone(), imported.clone(), variable));
        }
    }

    let mut tracked = BTreeMap::<SymbolId, (String, String, String, u32)>::new();
    for (module, imported, declarator) in initializers {
        context.check()?;
        let init_end = declarator.init.as_ref().ok_or(INVALID)?.span().end;
        match &declarator.id {
            BindingPattern::BindingIdentifier(id) => {
                let Some(symbol) = declared_symbol(&semantic, declarator.span, id.name.as_str())
                else {
                    continue;
                };
                if !semantic.scoping().symbol_is_mutated(symbol)
                    && !is_exported_binding(&semantic, symbol)
                {
                    tracked.insert(symbol, (module, imported, String::new(), init_end));
                }
            }
            BindingPattern::ObjectPattern(pattern) => {
                if pattern.rest.is_some() {
                    continue;
                }
                for property in &pattern.properties {
                    context.check()?;
                    if property.computed {
                        continue;
                    }
                    let PropertyKey::StaticIdentifier(key) = &property.key else {
                        continue;
                    };
                    let BindingPattern::BindingIdentifier(id) = &property.value else {
                        continue;
                    };
                    let Some(symbol) =
                        declared_symbol(&semantic, declarator.span, id.name.as_str())
                    else {
                        continue;
                    };
                    if semantic.scoping().symbol_is_mutated(symbol) {
                        continue;
                    }
                    tracked.insert(
                        symbol,
                        (
                            module.clone(),
                            imported.clone(),
                            key.name.to_string(),
                            init_end,
                        ),
                    );
                }
            }
            _ => {}
        }
    }

    for symbol in semantic.scoping().symbol_ids() {
        context.check()?;
        let Some((module, imported, destructured_property, init_end)) = tracked.get(&symbol) else {
            continue;
        };
        if semantic.scoping().symbol_is_mutated(symbol) {
            continue;
        }
        let local = semantic.scoping().symbol_name(symbol);
        let references = semantic.symbol_references(symbol).collect::<Vec<_>>();
        let mut resolved = Vec::new();
        let mut safe = true;
        for reference in references {
            context.check()?;
            let node_id = reference.node_id();
            let kind = semantic.nodes().parent_kind(node_id);
            let (reference_name, method, call_span) = match kind {
                AstKind::CallExpression(call) => {
                    let Expression::Identifier(callee) = &call.callee else {
                        safe = false;
                        continue;
                    };
                    if callee.name.as_str() != local
                        || destructured_property.is_empty()
                        || call.optional
                    {
                        safe = false;
                        continue;
                    }
                    (local.to_string(), destructured_property.clone(), call.span)
                }
                AstKind::StaticMemberExpression(member) => {
                    let Expression::Identifier(object) = &member.object else {
                        safe = false;
                        continue;
                    };
                    if object.name.as_str() != local {
                        safe = false;
                        continue;
                    }
                    let call = semantic
                        .nodes()
                        .parent_kind(semantic.nodes().parent_id(node_id));
                    let AstKind::CallExpression(call) = call else {
                        safe = false;
                        continue;
                    };
                    if call.optional || member.optional || call.callee.span() != member.span {
                        safe = false;
                        continue;
                    }
                    let property = member.property.name.to_string();
                    (format!("{local}.{property}"), property, call.span)
                }
                _ => {
                    safe = false;
                    continue;
                }
            };
            resolved.push((reference_name, method, call_span));
        }
        if !safe {
            continue;
        }
        for (reference_name, method, call_span) in resolved {
            context.check()?;
            if call_span.start <= *init_end {
                continue;
            }
            let (line, col) = positions.position(call_span.start as usize)?;
            let line = i64::try_from(line).map_err(|_| LIMIT)?;
            let col = i64::try_from(col).map_err(|_| LIMIT)?;
            let key = (reference_name, line, col);
            let value = (
                module.clone(),
                imported.clone(),
                language.to_owned(),
                method,
            );
            facts.reserved = facts
                .reserved
                .checked_add(key.0.len() + module.len() + imported.len() + language.len() + 64)
                .ok_or(LIMIT)?;
            if facts.calls.len() >= MAX_FACTS || facts.reserved > MAX_RESERVED {
                return Err(LIMIT);
            }
            if facts.calls.insert(key, value).is_some() {
                return Err(INVALID);
            }
        }
    }
    Ok(())
}

fn script_language(block: &Block) -> Option<&'static str> {
    match structure::attr(&block.attrs, "lang").unwrap_or("js") {
        "js" | "javascript" => Some("javascript"),
        "ts" | "typescript" => Some("typescript"),
        "jsx" => Some("jsx"),
        "tsx" => Some("tsx"),
        _ => None,
    }
}

pub(super) fn has_direct_eval(semantic: &Semantic<'_>) -> bool {
    semantic.nodes().iter().any(|node| {
        let AstKind::CallExpression(call) = node.kind() else {
            return false;
        };
        matches!(call.callee.get_inner_expression(), Expression::Identifier(identifier) if identifier.name == "eval")
    })
}

fn language_extension(language: &str) -> &'static str {
    match language {
        "javascript" => "js",
        "typescript" => "ts",
        "jsx" => "jsx",
        "tsx" => "tsx",
        _ => unreachable!(),
    }
}
