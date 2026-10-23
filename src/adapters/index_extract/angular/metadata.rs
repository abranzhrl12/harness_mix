use super::super::{CONTRACT, framework};
use super::template;
use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, Class, Declaration, Expression, ImportDeclarationSpecifier, ModuleExportName,
    ObjectPropertyKind, PropertyKey, Statement,
};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

const MAX_SOURCE: usize = 1024 * 1024;
const MAX_GRAPH_ROWS: usize = 20_000;
const LIMIT: Failure = (
    "EXTRACTION_LIMIT",
    "Angular metadata extraction limit exceeded",
);

#[derive(Default)]
struct Imports {
    named: BTreeMap<String, String>,
    namespaces: BTreeMap<String, String>,
}
enum Template {
    Inline { text: String, body: Range<usize> },
    External { path: String, literal: Range<usize> },
    Unsupported,
    Missing,
}
struct AngularClass {
    name: String,
    role: &'static str,
    class: Range<usize>,
    selector: Option<String>,
    template: Template,
}

fn imported_name(name: &ModuleExportName<'_>) -> Option<String> {
    match name {
        ModuleExportName::IdentifierName(id) => Some(id.name.to_string()),
        ModuleExportName::IdentifierReference(id) => Some(id.name.to_string()),
        ModuleExportName::StringLiteral(lit) => Some(lit.value.display().to_string()),
    }
}
fn imports(program: &oxc_ast::ast::Program<'_>) -> Imports {
    let mut result = Imports::default();
    for statement in &program.body {
        let Statement::ImportDeclaration(declaration) = statement else {
            continue;
        };
        if declaration.source.value != "@angular/core"
            || declaration.import_kind != oxc_ast::ast::ImportOrExportKind::Value
        {
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
                    if let Some(name) = imported_name(&specifier.imported) {
                        result.named.insert(specifier.local.name.to_string(), name);
                    }
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                    result
                        .namespaces
                        .insert(specifier.local.name.to_string(), "@angular/core".into());
                }
                _ => {}
            }
        }
    }
    result
}

fn decorator_name(expression: &Expression<'_>, imports: &Imports) -> Option<String> {
    let (base, member) = match expression {
        Expression::Identifier(id) => (id.name.as_str(), None),
        Expression::StaticMemberExpression(member) => {
            let Expression::Identifier(base) = &member.object else {
                return None;
            };
            (base.name.as_str(), Some(member.property.name.as_str()))
        }
        _ => return None,
    };
    if let Some(member) = member {
        imports
            .namespaces
            .contains_key(base)
            .then(|| member.to_owned())
    } else {
        imports.named.get(base).cloned()
    }
}

fn angular_decorator<'a>(
    decorator: &'a oxc_ast::ast::Decorator<'a>,
    imports: &Imports,
) -> Option<(&'static str, Option<&'a oxc_ast::ast::ObjectExpression<'a>>)> {
    let Expression::CallExpression(call) = &decorator.expression else {
        return None;
    };
    let name = decorator_name(&call.callee, imports)?;
    let name = match name.as_str() {
        "Component" => "Component",
        "Directive" => "Directive",
        "Injectable" => "Injectable",
        "Pipe" => "Pipe",
        "NgModule" => "NgModule",
        _ => return None,
    };
    let metadata = match call.arguments.first() {
        Some(Argument::ObjectExpression(object)) => Some(object.as_ref()),
        _ => None,
    };
    if name == "Component" && metadata.is_none() {
        return None;
    }
    Some((name, metadata))
}

fn literal(source: &str, expression: &Expression<'_>) -> Option<(String, Range<usize>)> {
    let span = expression.span();
    let range = span.start as usize..span.end as usize;
    let raw = source.get(range.clone())?;
    match expression {
        Expression::StringLiteral(value) => {
            let body = raw.get(1..raw.len().checked_sub(1)?)?;
            let quote = raw.as_bytes().first().copied()?;
            (matches!(quote, b'\'' | b'"')
                && raw.as_bytes().last().copied() == Some(quote)
                && !body.contains('\\')
                && value.value == body)
            .then(|| (body.to_owned(), range.start + 1..range.end - 1))
        }
        Expression::TemplateLiteral(value)
            if value.expressions.is_empty() && value.quasis.len() == 1 =>
        {
            let body = raw.get(1..raw.len().checked_sub(1)?)?;
            let cooked = value.quasis[0].value.cooked?.as_str()?;
            (raw.starts_with('`') && raw.ends_with('`') && !body.contains('\\') && body == cooked)
                .then(|| (body.to_owned(), range.start + 1..range.end - 1))
        }
        _ => None,
    }
}

fn metadata_properties<'a>(
    object: &'a oxc_ast::ast::ObjectExpression<'a>,
) -> Result<BTreeMap<String, &'a Expression<'a>>, ()> {
    let mut found = BTreeMap::new();
    for entry in &object.properties {
        let ObjectPropertyKind::ObjectProperty(entry) = entry else {
            return Err(());
        };
        if entry.computed || entry.method {
            return Err(());
        }
        let name = match &entry.key {
            PropertyKey::StaticIdentifier(name) => name.name.to_string(),
            PropertyKey::StringLiteral(name) => name.value.display().to_string(),
            _ => return Err(()),
        };
        if found.insert(name, &entry.value).is_some() {
            return Err(());
        }
    }
    Ok(found)
}

fn template_path(from: &str, value: &str) -> Option<String> {
    if value.is_empty()
        || value.starts_with('/')
        || value.contains(['\\', ':', '?', '#', '\0'])
        || value.contains("://")
    {
        return None;
    }
    let mut parts = from
        .rsplit_once('/')
        .map(|(base, _)| base.split('/').collect::<Vec<_>>())
        .unwrap_or_default();
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p if p.chars().any(char::is_control) => return None,
            p => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn classes<'a>(program: &'a oxc_ast::ast::Program<'a>) -> Vec<&'a Class<'a>> {
    program
        .body
        .iter()
        .filter_map(|statement| match statement {
            Statement::ClassDeclaration(class) => Some(class.as_ref()),
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::ClassDeclaration(class) => Some(class.as_ref()),
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

fn collect(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<(Vec<AngularClass>, Vec<String>), Failure> {
    if source.len() > MAX_SOURCE {
        return Err(LIMIT);
    }
    let lower = path.to_ascii_lowercase();
    let source_type = SourceType::from_path(&lower)
        .map_err(|_| ("SYNC_LANGUAGE", "Unsupported Angular metadata source"))?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    context.check()?;
    // The existing native TS extractor is authoritative. If Oxc cannot parse an otherwise accepted file,
    // preserve that graph unchanged; malformed Angular source is already rejected by the base extractor.
    if parsed.diagnostics.has_errors() {
        return Ok((Vec::new(), Vec::new()));
    }
    let imported = imports(&parsed.program);
    let mut result = Vec::new();
    let mut notices = Vec::new();
    for class in classes(&parsed.program) {
        context.check()?;
        let Some(name) = class.id.as_ref().map(|id| id.name.to_string()) else {
            continue;
        };
        let mut roles = BTreeSet::new();
        for decorator in &class.decorators {
            let Some((kind, metadata)) = angular_decorator(decorator, &imported) else {
                continue;
            };
            if !roles.insert(kind) {
                return Err((
                    "EXTRACTION_DEFERRED",
                    "Duplicate Angular decorator role cannot be projected safely",
                ));
            }
            if result.len() >= 256 {
                return Err(LIMIT);
            }
            if kind != "Component" {
                result.push(AngularClass {
                    name: name.clone(),
                    role: kind,
                    class: class.span.start as usize..class.span.end as usize,
                    selector: None,
                    template: Template::Missing,
                });
                continue;
            }
            let Some(metadata) = metadata else { continue };
            let properties = match metadata_properties(metadata) {
                Ok(properties) => properties,
                Err(()) => {
                    notices.push(format!("@Component({name}) uses computed, duplicate, or spread metadata; template fields were not projected"));
                    result.push(AngularClass {
                        name: name.clone(),
                        role: kind,
                        class: class.span.start as usize..class.span.end as usize,
                        selector: None,
                        template: Template::Unsupported,
                    });
                    continue;
                }
            };
            let selector = properties
                .get("selector")
                .and_then(|value| literal(source, value))
                .map(|(text, _)| text);
            let inline = properties
                .get("template")
                .map(|value| literal(source, value));
            let external = properties
                .get("templateUrl")
                .map(|value| literal(source, value));
            let template = match (inline, external) {
                (Some(Some((text, body))), None) => Template::Inline { text, body },
                (None, Some(Some((raw, literal)))) => match template_path(path, &raw) {
                    Some(path) => Template::External { path, literal },
                    None => {
                        notices.push(format!(
                            "@Component({name}) templateUrl is not a safe relative project path"
                        ));
                        Template::Unsupported
                    }
                },
                (None, None) => Template::Missing,
                _ => {
                    notices.push(format!(
                        "@Component({name}) template metadata is dynamic, escaped, or ambiguous"
                    ));
                    Template::Unsupported
                }
            };
            result.push(AngularClass {
                name: name.clone(),
                role: kind,
                class: class.span.start as usize..class.span.end as usize,
                selector,
                template,
            });
            if result.len() > 256 {
                return Err(LIMIT);
            }
        }
    }
    Ok((result, notices))
}

pub(in crate::adapters::index_extract) fn enrich(
    path: &str,
    source: &str,
    graph: &mut Value,
    context: &OperationContext,
) -> Result<(), Failure> {
    // Cheap prefilter only. Recognition is still based solely on Oxc import bindings below.
    if !source.contains('@') {
        return Ok(());
    }
    let (components, mut notices) = collect(path, source, context)?;
    if components.is_empty() {
        return Ok(());
    }
    let original_nodes = graph["nodes"].as_array().ok_or(CONTRACT)?;
    let original_edges = graph["edges"].as_array().ok_or(CONTRACT)?.len();
    let original_refs = graph["unresolvedReferences"]
        .as_array()
        .ok_or(CONTRACT)?
        .len();
    let language = original_nodes
        .iter()
        .find_map(|node| node["language"].as_str())
        .unwrap_or("typescript")
        .to_owned();
    let mut added_nodes = Vec::new();
    let mut added_edges = Vec::new();
    let mut added_refs = Vec::new();
    let positions = framework::Positions::new(source);
    for component in components {
        context.check()?;
        let node_kind = if component.role == "Component" {
            "component"
        } else {
            "module"
        };
        let role_key = component.role.to_ascii_lowercase();
        let id = framework::id("angular", path, &role_key, component.class.start);
        let (sl, sc) = positions.position(component.class.start)?;
        let (el, ec) = positions.position(component.class.end)?;
        let mut class_candidates = original_nodes
            .iter()
            .filter(|node| node["kind"] == "class" && node["name"] == component.name);
        let first_class = class_candidates.next();
        let class_id = if class_candidates.next().is_none() {
            first_class
                .and_then(|node| node["id"].as_str())
                .map(str::to_owned)
        } else {
            notices.push(format!(
                "Class {} has ambiguous graph anchors; decorator edge omitted",
                component.name
            ));
            None
        };
        let signature = if component.role == "Component" {
            format!(
                "@Component{} class {}",
                component
                    .selector
                    .as_ref()
                    .map(|s| format!(" selector=\"{s}\""))
                    .unwrap_or_default(),
                component.name
            )
        } else {
            format!("@{} class {}", component.role, component.name)
        };
        let mut node = json!({"id":id,"kind":node_kind,"name":component.name,"qualifiedName":format!("{path}::{}#{}",component.name,component.role),"filePath":path,"language":language,"startLine":sl,"endLine":el,"startColumn":sc,"endColumn":ec,"signature":signature,"decorators":[component.role]});
        if let Some(selector) = component.selector {
            node["selector"] = json!(selector);
        }
        added_nodes.push(node);
        if let Some(file_id) = original_nodes
            .iter()
            .find(|node| node["kind"] == "file")
            .and_then(|node| node["id"].as_str())
        {
            added_edges.push(
                json!({"source":file_id,"target":id,"kind":"contains","provenance":"heuristic"}),
            );
        }
        if let Some(target) = class_id {
            added_edges.push(json!({"source":id,"target":target,"kind":"decorates","provenance":"heuristic","metadata":{"framework":"angular","decorator":component.role}}));
        }
        if component.role != "Component" {
            continue;
        }
        match component.template {
            Template::External {
                path: target,
                literal,
            } => {
                let (line, column) = positions.position(literal.start)?;
                added_refs.push(json!({"fromNodeId":id,"referenceName":target,"referenceKind":"framework_template","line":line,"column":column,"filePath":path}));
            }
            Template::Inline { text, body } => {
                match template::inline(path, source, &text, body.start, &id, &language, context) {
                    Ok((mut ns, mut es, mut ts)) => {
                        added_nodes.append(&mut ns);
                        added_edges.append(&mut es);
                        notices.append(&mut ts);
                    }
                    Err(error) if matches!(error.0, "CANCELLED" | "TIMEOUT") => return Err(error),
                    Err(_) => notices.push(format!(
                        "Inline template for {} was not structurally projected",
                        component.name
                    )),
                }
            }
            Template::Unsupported => notices.push(format!(
                "Template structure for {} was not projected",
                component.name
            )),
            Template::Missing => {}
        }
        if original_nodes.len().saturating_add(added_nodes.len()) > MAX_GRAPH_ROWS
            || original_edges.saturating_add(added_edges.len()) > MAX_GRAPH_ROWS
            || original_refs.saturating_add(added_refs.len()) > MAX_GRAPH_ROWS
        {
            return Err(LIMIT);
        }
    }
    graph["nodes"]
        .as_array_mut()
        .ok_or(CONTRACT)?
        .append(&mut added_nodes);
    graph["edges"]
        .as_array_mut()
        .ok_or(CONTRACT)?
        .append(&mut added_edges);
    graph["unresolvedReferences"]
        .as_array_mut()
        .ok_or(CONTRACT)?
        .append(&mut added_refs);
    graph["coverage"] = json!({"parser":"angular-metadata-structural-subset-v1","limitations":["Angular compiler, template type checking, dependency injection and runtime semantics are not evaluated","Angular decorators require direct named or namespace imports from @angular/core; full lexical shadowing is not modeled","Only literal @Component selector/template/templateUrl metadata is projected"],"notices":notices});
    Ok(())
}
