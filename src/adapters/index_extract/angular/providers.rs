//! Static Angular provider registration subset (`useClass` / `useExisting`).
//!
//! A registration is a source fact. It does not claim which provider an Angular
//! injector selects at runtime.
mod bindings;
mod presentation;
use self::bindings::{classes, decorator_role, imported_value, properties};
use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_ast::ast::{Argument, ArrayExpressionElement, Expression};
use oxc_parser::Parser;
use oxc_semantic::{Semantic, SemanticBuilder};
use oxc_span::SourceType;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const LIMIT: Failure = (
    "EXTRACTION_LIMIT",
    "Angular provider registration extraction limit exceeded",
);
const CONTRACT: Failure = (
    "EXTRACTION_CONTRACT",
    "Angular provider registration graph contract is invalid",
);
const MAX_SOURCE: usize = 2 * 1024 * 1024;
const MAX_REGISTRATIONS: usize = 10_000;

#[derive(Clone)]
struct Reference {
    name: String,
    line: usize,
    column: usize,
    module: String,
    imported: String,
}

#[derive(Clone)]
struct Registration {
    class_name: String,
    class_start: usize,
    provider_start: usize,
    provider_end: usize,
    token: Reference,
    implementation: Reference,
    form: &'static str,
}

#[derive(Clone, Debug)]
pub(in crate::adapters) struct Binding {
    pub module: String,
    pub imported: String,
    pub relationship: String,
    pub registration_form: String,
}

#[derive(Default)]
pub(in crate::adapters) struct Facts {
    sites: BTreeMap<(String, i64, i64, String), Binding>,
    registrations: Vec<Registration>,
    notices: BTreeSet<String>,
    pub reserved: usize,
}

impl Facts {
    pub(in crate::adapters) fn binding(
        &self,
        name: &str,
        line: i64,
        col: i64,
        kind: &str,
    ) -> Option<&Binding> {
        self.sites
            .get(&(name.to_owned(), line, col, kind.to_owned()))
    }
}

fn reference(
    semantic: &Semantic<'_>,
    positions: &super::super::framework::Positions<'_>,
    expression: &Expression<'_>,
) -> Option<Reference> {
    let Expression::Identifier(identifier) = expression else {
        return None;
    };
    let binding = imported_value(
        semantic,
        identifier.name.as_str(),
        identifier.span.start,
        identifier.span.end,
    )?;
    let (line, column) = positions.position(identifier.span.start as usize).ok()?;
    Some(Reference {
        name: identifier.name.to_string(),
        line,
        column,
        module: binding.module,
        imported: binding.imported,
    })
}

fn provider_array<'a>(
    semantic: &Semantic<'_>,
    positions: &super::super::framework::Positions<'_>,
    array: &'a oxc_ast::ast::ArrayExpression<'a>,
    class_name: &str,
    class_start: usize,
    registrations: &mut Vec<Registration>,
    notices: &mut BTreeSet<String>,
    context: &OperationContext,
) -> Result<(), Failure> {
    let mut local = Vec::new();
    let mut unsupported_entry = false;
    for item in &array.elements {
        context.check()?;
        let ArrayExpressionElement::ObjectExpression(object) = item else {
            unsupported_entry = true;
            notices.insert(format!("Provider scope {class_name} contains a spread or dynamic entry; that entry was skipped"));
            continue;
        };
        let fields = match properties(object) {
            Ok(fields) => fields,
            Err(()) => {
                unsupported_entry = true;
                notices.insert(format!("Provider scope {class_name} contains computed, duplicate, method, or spread fields; that entry was skipped"));
                continue;
            }
        };
        if fields.keys().any(|key| {
            !matches!(
                key.as_str(),
                "provide" | "useClass" | "useExisting" | "multi"
            )
        }) {
            unsupported_entry = true;
            notices.insert(format!("Provider scope {class_name} contains factory, value, dependency, or unsupported fields; that entry was skipped"));
            continue;
        }
        let Some(provide) = fields.get("provide") else {
            unsupported_entry = true;
            notices.insert(format!("Provider scope {class_name} contains an entry without a static provide token; that entry was skipped"));
            continue;
        };
        let use_class = fields.get("useClass");
        let use_existing = fields.get("useExisting");
        let (implementation, form) = match (use_class, use_existing) {
            (Some(value), None) => (*value, "useClass"),
            (None, Some(value)) => (*value, "useExisting"),
            _ => {
                unsupported_entry = true;
                notices.insert(format!("Provider scope {class_name} has an ambiguous or unsupported implementation form; that entry was skipped"));
                continue;
            }
        };
        if fields.get("multi").is_some_and(
            |value| !matches!(&value.value, Expression::BooleanLiteral(boolean) if !boolean.value),
        ) {
            unsupported_entry = true;
            notices.insert(format!("Provider scope {class_name} has a multi or dynamic provider; that entry was skipped"));
            continue;
        }
        let Some(token) = reference(semantic, positions, &provide.value) else {
            unsupported_entry = true;
            notices.insert(format!("Provider scope {class_name} uses a non-imported or dynamic token; that entry was skipped"));
            continue;
        };
        let Some(implementation) = reference(semantic, positions, &implementation.value) else {
            unsupported_entry = true;
            notices.insert(format!("Provider scope {class_name} uses a non-imported or dynamic implementation; that entry was skipped"));
            continue;
        };
        local.push(Registration {
            class_name: class_name.to_owned(),
            class_start,
            provider_start: object.span.start as usize,
            provider_end: object.span.end as usize,
            token,
            implementation,
            form,
        });
    }
    if unsupported_entry {
        return Ok(());
    }
    let mut counts = BTreeMap::<(String, String), usize>::new();
    for item in &local {
        *counts
            .entry((item.token.module.clone(), item.token.imported.clone()))
            .or_default() += 1;
    }
    for item in local {
        if counts[&(item.token.module.clone(), item.token.imported.clone())] != 1 {
            notices.insert(format!("Provider scope {class_name} registers the same static token more than once; duplicate registrations were skipped"));
            continue;
        }
        registrations.push(item);
        if registrations.len() > MAX_REGISTRATIONS {
            return Err(LIMIT);
        }
    }
    Ok(())
}

fn collect(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<(Vec<Registration>, BTreeSet<String>), Failure> {
    if source.len() > MAX_SOURCE {
        return Err(LIMIT);
    }
    let source_type = SourceType::from_path(path.to_ascii_lowercase()).map_err(|_| {
        (
            "EXTRACTION_DEFERRED",
            "Angular provider source type is unsupported",
        )
    })?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    context.check()?;
    if parsed.diagnostics.has_errors() {
        return Ok((Vec::new(), BTreeSet::new()));
    }
    let built = SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(true)
        .build(&parsed.program);
    if built.diagnostics.has_errors() {
        return Ok((Vec::new(), BTreeSet::new()));
    }
    if built.semantic.scoping().symbols_len() > 20_000 {
        return Err(LIMIT);
    }
    let semantic = built.semantic;
    let positions = super::super::framework::Positions::new(source);
    let mut registrations = Vec::new();
    let mut notices = BTreeSet::new();
    for class in classes(&parsed.program) {
        context.check()?;
        let Some(class_name) = class.id.as_ref().map(|id| id.name.to_string()) else {
            continue;
        };
        let roles = class
            .decorators
            .iter()
            .filter_map(|decorator| decorator_role(&semantic, &decorator.expression))
            .collect::<BTreeSet<_>>();
        if roles.len() != 1 {
            continue;
        }
        let role = *roles.iter().next().ok_or(CONTRACT)?;
        let matched_decorators = class
            .decorators
            .iter()
            .filter(|decorator| decorator_role(&semantic, &decorator.expression) == Some(role))
            .count();
        if matched_decorators != 1 {
            notices.insert(format!("Class {class_name} has duplicate Angular provider decorators; registrations were not projected"));
            continue;
        }
        for decorator in &class.decorators {
            if decorator_role(&semantic, &decorator.expression) != Some(role) {
                continue;
            }
            let Expression::CallExpression(call) = &decorator.expression else {
                continue;
            };
            let Some(Argument::ObjectExpression(metadata)) = call.arguments.first() else {
                continue;
            };
            let fields = match properties(metadata) {
                Ok(fields) => fields,
                Err(()) => {
                    notices.insert(format!("@{role}({class_name}) metadata is computed, duplicated, or spread; providers were not projected"));
                    continue;
                }
            };
            if let Some(providers) = fields.get("providers") {
                let Expression::ArrayExpression(array) = &providers.value else {
                    notices.insert(format!("@{role}({class_name}) providers is dynamic; registrations were not projected"));
                    continue;
                };
                provider_array(
                    &semantic,
                    &positions,
                    array,
                    &class_name,
                    class.span.start as usize,
                    &mut registrations,
                    &mut notices,
                    context,
                )?;
            }
        }
    }
    Ok((registrations, notices))
}

pub(in crate::adapters) fn analyze(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Facts, Failure> {
    let (registrations, notices) = collect(path, source, context)?;
    let mut facts = Facts {
        registrations,
        notices,
        ..Facts::default()
    };
    for registration in &facts.registrations {
        for (binding, relationship, kind) in [
            (&registration.token, "token", "angular_provider_token"),
            (
                &registration.implementation,
                "implementation",
                "angular_provider_implementation",
            ),
        ] {
            facts.reserved = facts.reserved.saturating_add(
                binding.name.len()
                    + binding.module.len()
                    + binding.imported.len()
                    + kind.len()
                    + 64,
            );
            if facts.sites.len() >= MAX_REGISTRATIONS * 2 || facts.reserved > 16 * 1024 * 1024 {
                return Err(LIMIT);
            }
            let value = Binding {
                module: binding.module.clone(),
                imported: binding.imported.clone(),
                relationship: relationship.to_owned(),
                registration_form: registration.form.to_owned(),
            };
            if facts
                .sites
                .insert(
                    (
                        binding.name.clone(),
                        binding.line as i64,
                        binding.column as i64,
                        kind.to_owned(),
                    ),
                    value,
                )
                .is_some()
            {
                return Err((
                    "EXTRACTION_DEFERRED",
                    "Angular provider relationship coordinates are ambiguous",
                ));
            }
        }
    }
    Ok(facts)
}

pub(super) fn enrich(
    path: &str,
    source: &str,
    graph: &mut Value,
    context: &OperationContext,
) -> Result<(), Failure> {
    presentation::enrich(path, source, graph, context)
}
