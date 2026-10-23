//! Proven Angular DI token sites. Provider visibility and runtime injector
//! selection are intentionally left to runtime-aware tooling.
mod bindings;
mod presentation;

use self::bindings::{imported_token, is_core_decorator, is_core_inject};
use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_ast::{
    AstKind,
    ast::{
        Argument, Class, ClassElement, Expression, MethodDefinitionKind, Statement, TSType,
        TSTypeName,
    },
};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const LIMIT: Failure = ("EXTRACTION_LIMIT", "Angular DI extraction limit exceeded");
const INVALID: Failure = (
    "EXTRACTION_DEFERRED",
    "Angular DI import bindings could not be established",
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::adapters) struct Binding {
    pub module: String,
    pub imported: String,
    pub token_form: String,
    pub class_name: String,
    pub class_line: i64,
    pub class_col: i64,
}

#[derive(Default)]
pub(in crate::adapters) struct Facts {
    sites: BTreeMap<(String, i64, i64), Binding>,
    pub reserved: usize,
}

impl Facts {
    pub fn binding(&self, name: &str, line: i64, col: i64) -> Option<&Binding> {
        self.sites.get(&(name.to_owned(), line, col))
    }
}

#[derive(Clone)]
struct Candidate<'a> {
    class: &'a Class<'a>,
    name: String,
    allowed_calls: Vec<std::ops::Range<usize>>,
    constructor_functions: BTreeSet<(usize, usize)>,
}

fn classes<'a>(program: &'a oxc_ast::ast::Program<'a>) -> Vec<&'a Class<'a>> {
    program
        .body
        .iter()
        .filter_map(|statement| match statement {
            Statement::ClassDeclaration(class) => Some(class.as_ref()),
            Statement::ExportDeclaration(export) => match &export.declaration {
                oxc_ast::ast::Declaration::ClassDeclaration(class) => Some(class.as_ref()),
                _ => None,
            },
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                    Some(class.as_ref())
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn eligible_classes<'a>(
    program: &'a oxc_ast::ast::Program<'a>,
    semantic: &oxc_semantic::Semantic<'a>,
) -> Vec<Candidate<'a>> {
    let mut result = Vec::new();
    for class in classes(program) {
        let Some(name) = class.id.as_ref().map(|id| id.name.to_string()) else {
            continue;
        };
        let decorated = class.decorators.iter().any(|decorator| {
            is_core_decorator(
                semantic,
                &decorator.expression,
                &["Component", "Injectable", "Directive", "Pipe", "NgModule"],
            )
        });
        if !decorated {
            continue;
        }
        let mut allowed_calls = Vec::new();
        let mut constructor_functions = BTreeSet::new();
        for element in &class.body.body {
            match element {
                ClassElement::PropertyDefinition(field) if !field.r#static => {
                    if let Some(value) = &field.value {
                        allowed_calls.push(value.span().start as usize..value.span().end as usize);
                    }
                }
                ClassElement::MethodDefinition(method)
                    if method.kind == MethodDefinitionKind::Constructor =>
                {
                    if let Some(body) = &method.value.body {
                        allowed_calls.push(body.span.start as usize..body.span.end as usize);
                        constructor_functions.insert((
                            method.value.span.start as usize,
                            method.value.span.end as usize,
                        ));
                    }
                }
                _ => {}
            }
        }
        result.push(Candidate {
            class,
            name,
            allowed_calls,
            constructor_functions,
        });
    }
    result
}

fn add_site(
    facts: &mut Facts,
    positions: &super::super::framework::Positions<'_>,
    name: &str,
    span: oxc_span::Span,
    owner_name: &str,
    owner_span: oxc_span::Span,
    module: String,
    imported: String,
    token_form: &str,
) -> Result<(), Failure> {
    let (line, col) = positions.position(span.start as usize)?;
    let (class_line, class_col) = positions.position(owner_span.start as usize)?;
    facts.reserved += name.len() + module.len() + imported.len() + token_form.len() + 64;
    if facts.sites.len() >= 20_000 || facts.reserved > 16 * 1024 * 1024 {
        return Err(LIMIT);
    }
    facts.sites.insert(
        (name.to_owned(), line as i64, col as i64),
        Binding {
            module,
            imported,
            token_form: token_form.to_owned(),
            class_name: owner_name.to_owned(),
            class_line: class_line as i64,
            class_col: class_col as i64,
        },
    );
    Ok(())
}

fn token_type<'a>(
    parameter: &'a oxc_ast::ast::FormalParameter<'a>,
) -> Option<&'a oxc_ast::ast::IdentifierReference<'a>> {
    let annotation = parameter.type_annotation.as_ref()?;
    let TSType::TSTypeReference(reference) = &annotation.type_annotation else {
        return None;
    };
    let TSTypeName::IdentifierReference(identifier) = &reference.type_name else {
        return None;
    };
    Some(identifier)
}

fn explicit_inject<'a>(
    parameter: &'a oxc_ast::ast::FormalParameter<'a>,
    semantic: &oxc_semantic::Semantic<'a>,
) -> (
    bool,
    Option<(&'a oxc_ast::ast::IdentifierReference<'a>, String, String)>,
) {
    let mut saw_inject = false;
    let mut token = None;
    for decorator in &parameter.decorators {
        if !is_core_decorator(semantic, &decorator.expression, &["Inject"]) {
            continue;
        }
        let Expression::CallExpression(call) = &decorator.expression else {
            continue;
        };
        saw_inject = true;
        if let Some(Argument::Identifier(identifier)) = call.arguments.first() {
            if let Some((module, imported)) = imported_token(
                semantic,
                identifier.name.as_str(),
                identifier.span.start,
                identifier.span.end,
            ) {
                token = Some((identifier.as_ref(), module, imported));
            }
        }
    }
    (saw_inject, token)
}

pub(in crate::adapters) fn analyze(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Facts, Failure> {
    context.check()?;
    if !source.contains("@angular/core") {
        return Ok(Facts::default());
    }
    if source.len() > super::super::super::index_sync::MAX_SOURCE_BYTES {
        return Err(LIMIT);
    }
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path.to_ascii_lowercase()).map_err(|_| INVALID)?;
    let parsed = Parser::new(&allocator, source, source_type).parse();
    context.check()?;
    if parsed.diagnostics.has_errors() {
        return Ok(Facts::default());
    }
    let semantic_built = SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(true)
        .build(&parsed.program);
    if semantic_built.diagnostics.has_errors() {
        return Ok(Facts::default());
    }
    let semantic = semantic_built.semantic;
    if semantic.scoping().symbols_len() > 20_000 {
        return Err(LIMIT);
    }
    let candidates = eligible_classes(&parsed.program, &semantic);
    let mut facts = Facts::default();
    let positions = super::super::framework::Positions::new(source);

    for candidate in &candidates {
        for element in &candidate.class.body.body {
            let ClassElement::MethodDefinition(method) = element else {
                continue;
            };
            if method.kind != MethodDefinitionKind::Constructor {
                continue;
            }
            for parameter in &method.value.params.items {
                context.check()?;
                let (saw_inject, explicit) = explicit_inject(parameter, &semantic);
                if saw_inject {
                    if let Some((identifier, module, imported)) = explicit {
                        add_site(
                            &mut facts,
                            &positions,
                            identifier.name.as_str(),
                            identifier.span,
                            &candidate.name,
                            candidate.class.span,
                            module,
                            imported,
                            "inject_decorator",
                        )?;
                    }
                    continue;
                }
                if let Some(identifier) = token_type(parameter)
                    && let Some((module, imported)) = imported_token(
                        &semantic,
                        identifier.name.as_str(),
                        identifier.span.start,
                        identifier.span.end,
                    )
                {
                    add_site(
                        &mut facts,
                        &positions,
                        identifier.name.as_str(),
                        identifier.span,
                        &candidate.name,
                        candidate.class.span,
                        module,
                        imported,
                        "constructor_type",
                    )?;
                }
            }
        }
    }

    for symbol in semantic.scoping().symbol_ids() {
        context.check()?;
        if semantic.scoping().symbol_flags(symbol).is_type_import()
            || !matches!(
                semantic.symbol_declaration(symbol).kind(),
                AstKind::ImportSpecifier(_)
            )
        {
            continue;
        }
        let declaration = semantic.symbol_declaration(symbol);
        let AstKind::ImportSpecifier(specifier) = declaration.kind() else {
            continue;
        };
        if specifier.import_kind != oxc_ast::ast::ImportOrExportKind::Value {
            continue;
        }
        let imported = match &specifier.imported {
            oxc_ast::ast::ModuleExportName::IdentifierName(id) => id.name.as_str(),
            oxc_ast::ast::ModuleExportName::IdentifierReference(id) => id.name.as_str(),
            oxc_ast::ast::ModuleExportName::StringLiteral(literal) => literal.value.as_str().unwrap_or(""),
        };
        if imported != "inject" {
            continue;
        }
        let inject_name = semantic.scoping().symbol_name(symbol);
        for reference in semantic.symbol_references(symbol) {
            context.check()?;
            let call_node = semantic.nodes().parent_node(reference.node_id());
            let AstKind::CallExpression(call) = call_node.kind() else {
                continue;
            };
            let Expression::Identifier(callee) = &call.callee else {
                continue;
            };
            let reference_span = semantic.nodes().get_node(reference.node_id()).kind().span();
            if callee.name.as_str() != inject_name
                || callee.span.start != reference_span.start
                || callee.span.end != reference_span.end
                || !is_core_inject(&semantic, inject_name, callee.span.start, callee.span.end)
            {
                continue;
            }
            let Some(candidate) = candidates.iter().find(|candidate| {
                let call_range = call.span.start as usize..call.span.end as usize;
                candidate.class.span.start as usize <= call_range.start
                    && call_range.end <= candidate.class.span.end as usize
                    && candidate.allowed_calls.iter().any(|allowed| {
                        allowed.start <= call_range.start && call_range.end <= allowed.end
                    })
            }) else {
                continue;
            };
            let direct_context = semantic
                .nodes()
                .ancestors(call_node.id())
                .take_while(|node| {
                    let span = node.kind().span();
                    !(span.start as usize <= candidate.class.span.start as usize
                        && candidate.class.span.end as usize <= span.end as usize
                        && matches!(node.kind(), AstKind::Class(_)))
                })
                .all(|node| match node.kind() {
                    AstKind::Class(_) => false,
                    AstKind::ArrowFunctionExpression(arrow) => candidate
                        .constructor_functions
                        .contains(&(arrow.span.start as usize, arrow.span.end as usize)),
                    AstKind::Function(function) => candidate
                        .constructor_functions
                        .contains(&(function.span.start as usize, function.span.end as usize)),
                    _ => true,
                });
            if !direct_context {
                continue;
            }
            let Some(Argument::Identifier(token)) = call.arguments.first() else {
                continue;
            };
            let Some((module, imported)) = imported_token(
                &semantic,
                token.name.as_str(),
                token.span.start,
                token.span.end,
            ) else {
                continue;
            };
            add_site(
                &mut facts,
                &positions,
                token.name.as_str(),
                token.span,
                &candidate.name,
                candidate.class.span,
                module,
                imported,
                "inject_call",
            )?;
        }
    }
    Ok(facts)
}

pub(in crate::adapters::index_extract) fn enrich(
    path: &str,
    source: &str,
    graph: &mut Value,
    context: &OperationContext,
) -> Result<(), Failure> {
    presentation::enrich(path, source, graph, context)
}
