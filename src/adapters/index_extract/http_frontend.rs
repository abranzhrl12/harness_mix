//! Static frontend HTTP route extraction from literal request URLs.
use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_ast::{
    AstKind,
    ast::{CallExpression, Expression, ObjectPropertyKind, PropertyKey, StaticMemberExpression},
};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType};
use serde_json::{Value, json};

const LIMIT: Failure = (
    "EXTRACTION_LIMIT",
    "Frontend HTTP extraction limit exceeded",
);
const CONTRACT: Failure = (
    "EXTRACTION_CONTRACT",
    "Invalid frontend HTTP extraction graph",
);

#[derive(Clone)]
struct Request {
    url: String,
    method: String,
    at: usize,
}

struct Calls<'a> {
    source: &'a str,
    requests: Vec<Request>,
    count: usize,
}
impl<'a> Calls<'a> {
    fn add(
        &mut self,
        call: &CallExpression<'_>,
        angular_clients: &super::http_angular::Clients,
    ) {
        self.count += 1;
        if self.count > 20_000 {
            return;
        }
        let Some((url, mut method)) = request(self.source, call)
            .or_else(|| super::http_angular::request(self.source, call, angular_clients))
        else {
            return;
        };
        if url.is_empty() || url.contains(['\\', '\0']) {
            return;
        }
        method.make_ascii_uppercase();
        if !matches!(
            method.as_str(),
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
        ) {
            return;
        }
        self.requests.push(Request {
            url,
            method,
            at: call.span.start as usize,
        });
    }
}
fn string<'a>(source: &'a str, expr: &Expression<'_>) -> Option<&'a str> {
    let raw = source.get(expr.span().start as usize..expr.span().end as usize)?;
    match expr {
        Expression::StringLiteral(value) => {
            let quote = *raw.as_bytes().first()?;
            if !matches!(quote, b'\'' | b'"') || raw.as_bytes().last().copied() != Some(quote) {
                return None;
            }
            let inner = raw.get(1..raw.len().checked_sub(1)?)?;
            (!inner.contains('\\') && value.value == inner).then_some(inner)
        }
        Expression::TemplateLiteral(value)
            if value.expressions.is_empty() && value.quasis.len() == 1 =>
        {
            let inner = raw.get(1..raw.len().checked_sub(1)?)?;
            let cooked = value.quasis[0].value.cooked?.as_str()?;
            (raw.starts_with('\x60')
                && raw.ends_with('\x60')
                && !inner.contains('\\')
                && inner == cooked)
                .then_some(inner)
        }
        _ => None,
    }
}
fn member_name<'a, 'b>(member: &'a StaticMemberExpression<'b>) -> Option<(&'a str, &'a str)> {
    let Expression::Identifier(object) = &member.object else {
        return None;
    };
    Some((object.name.as_str(), member.property.name.as_str()))
}
fn safe_object(object: &oxc_ast::ast::ObjectExpression<'_>) -> bool {
    let mut keys = std::collections::BTreeSet::new();
    object.properties.iter().all(|entry| {
        let ObjectPropertyKind::ObjectProperty(property) = entry else {
            return false;
        };
        if property.computed || property.method {
            return false;
        }
        let key = match &property.key {
            PropertyKey::StaticIdentifier(id) => id.name.as_str(),
            PropertyKey::StringLiteral(value) => {
                let Some(text) = value.value.as_str() else {
                    return false;
                };
                text
            }
            _ => return false,
        };
        keys.insert(key)
    })
}
fn prop<'a>(
    object: &'a oxc_ast::ast::ObjectExpression<'a>,
    name: &str,
) -> Option<&'a Expression<'a>> {
    object.properties.iter().find_map(|entry| {
        let ObjectPropertyKind::ObjectProperty(property) = entry else {
            return None;
        };
        let key = match &property.key {
            PropertyKey::StaticIdentifier(id) => id.name.as_str(),
            PropertyKey::StringLiteral(value) => value.value.as_str()?,
            _ => return None,
        };
        (key == name).then_some(&property.value)
    })
}
fn request(source: &str, call: &CallExpression<'_>) -> Option<(String, String)> {
    if let Expression::StaticMemberExpression(member) = &call.callee {
        if let Some((object, verb)) = member_name(member.as_ref()) {
            if object == "axios"
                && matches!(
                    verb,
                    "get" | "post" | "put" | "patch" | "delete" | "head" | "options"
                )
            {
                let url = call
                    .arguments
                    .first()?
                    .as_expression()
                    .and_then(|e| string(source, e))?;
                return Some((url.to_owned(), verb.to_owned()));
            }
            if object == "$" && matches!(verb, "get" | "post" | "getJSON") {
                let url = call
                    .arguments
                    .first()?
                    .as_expression()
                    .and_then(|e| string(source, e))?;
                return Some((
                    url.to_owned(),
                    if verb == "post" { "POST" } else { "GET" }.to_owned(),
                ));
            }
            if object == "$" && verb == "ajax" {
                let Expression::ObjectExpression(config) =
                    call.arguments.first()?.as_expression()?
                else {
                    return None;
                };
                if !safe_object(config) {
                    return None;
                }
                let url = prop(config, "url").and_then(|e| string(source, e))?;
                let method_prop = prop(config, "method").or_else(|| prop(config, "type"));
                let method = if let Some(expression) = method_prop {
                    string(source, expression)?
                } else {
                    "GET"
                };
                return Some((url.to_owned(), method.to_owned()));
            }
        }
    }
    if matches!(&call.callee, Expression::Identifier(id) if id.name == "fetch") {
        let url = call
            .arguments
            .first()?
            .as_expression()
            .and_then(|e| string(source, e))?;
        let method = match call.arguments.get(1).and_then(|a| a.as_expression()) {
            None => "GET",
            Some(Expression::ObjectExpression(options)) => {
                if !safe_object(options) {
                    return None;
                }
                if let Some(value) = prop(options, "method") {
                    string(source, value)?
                } else {
                    "GET"
                }
            }
            Some(_) => return None,
        };
        return Some((url.to_owned(), method.to_owned()));
    }
    None
}

pub(super) fn enrich(
    path: &str,
    source: &str,
    graph: &mut Value,
    context: &OperationContext,
) -> Result<(), Failure> {
    context.check()?;
    let lower_path = path.to_ascii_lowercase();
    let mut regions: Vec<(std::ops::Range<usize>, &'static str)> = Vec::new();
    if lower_path.ends_with(".vue") {
        let document = crate::adapters::parsers::vue::structure::parse(
            source,
            &harness_domain::hash(source.as_bytes()),
            context,
        )?;
        for block in document.blocks.into_iter().filter(|block| {
            block.kind == "script"
                && crate::adapters::parsers::vue::structure::attr(&block.attrs, "src").is_none()
        }) {
            let ext = match crate::adapters::parsers::vue::structure::attr(&block.attrs, "lang")
                .unwrap_or("js")
            {
                "js" | "javascript" => "js",
                "ts" | "typescript" => "ts",
                "jsx" => "jsx",
                "tsx" => "tsx",
                _ => continue,
            };
            regions.push((block.body, ext));
        }
    } else {
        let ext = if lower_path.ends_with(".tsx") {
            "tsx"
        } else if lower_path.ends_with(".jsx") {
            "jsx"
        } else if [".ts", ".mts", ".cts"]
            .iter()
            .any(|extension| lower_path.ends_with(extension))
        {
            "ts"
        } else {
            "js"
        };
        regions.push((0..source.len(), ext));
    }
    let positions = super::framework::Positions::new(source);
    let nodes = graph["nodes"].as_array().ok_or(CONTRACT)?;
    let file_id = nodes
        .iter()
        .find(|n| n["kind"] == "file")
        .and_then(|n| n["id"].as_str())
        .ok_or(CONTRACT)?
        .to_owned();
    let mut references = Vec::new();
    for (region, ext) in regions {
        context.check()?;
        let body = source.get(region.clone()).ok_or(CONTRACT)?;
        let source_type = SourceType::from_path(format!("source.{ext}"))
            .map_err(|_| ("SYNC_LANGUAGE", "Unsupported frontend source"))?;
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, body, source_type).parse();
        if parsed.diagnostics.has_errors() {
            continue;
        }
        let built = SemanticBuilder::new()
            .with_build_nodes(true)
            .build(&parsed.program);
        let semantic = built.semantic;
        let angular_clients = super::http_angular::Clients::new(&parsed.program, &semantic);
        let mut calls = Calls {
            source: body,
            requests: Vec::new(),
            count: 0,
        };
        for node in semantic.nodes().iter() {
            context.check()?;
            if let AstKind::CallExpression(call) = node.kind() {
                calls.add(call, &angular_clients);
            }
        }
        if calls.count > 20_000 {
            return Err(LIMIT);
        }
        for call in calls.requests {
            context.check()?;
            let (line, column) =
                positions.position(region.start.checked_add(call.at).ok_or(CONTRACT)?)?;
            let owner = nodes
                .iter()
                .filter(|n| matches!(n["kind"].as_str(), Some("function" | "method")))
                .filter(|n| contains(n, line, column))
                .min_by_key(|n| span_size(n))
                .and_then(|n| n["id"].as_str())
                .unwrap_or(&file_id);
            let reference_name = json!({"url":call.url,"method":call.method}).to_string();
            references.push(json!({"fromNodeId":owner,"referenceName":reference_name,
                "referenceKind":"http_route","line":line,"column":column,"filePath":path}));
        }
    }
    graph["unresolvedReferences"]
        .as_array_mut()
        .ok_or(CONTRACT)?
        .extend(references);
    Ok(())
}
fn contains(n: &Value, line: usize, col: usize) -> bool {
    let sl = n["startLine"].as_u64().unwrap_or(0) as usize;
    let el = n["endLine"].as_u64().unwrap_or(0) as usize;
    let sc = n["startColumn"].as_u64().unwrap_or(0) as usize;
    let ec = n["endColumn"].as_u64().unwrap_or(u64::MAX) as usize;
    (line > sl || line == sl && col >= sc) && (line < el || line == el && col < ec)
}
fn span_size(n: &Value) -> (u64, u64) {
    let lines = n["endLine"]
        .as_u64()
        .unwrap_or(u64::MAX)
        .saturating_sub(n["startLine"].as_u64().unwrap_or(0));
    let columns = n["endColumn"]
        .as_u64()
        .unwrap_or(u64::MAX)
        .saturating_sub(n["startColumn"].as_u64().unwrap_or(0));
    (lines, columns)
}

#[cfg(test)]
#[path = "http_frontend_tests.rs"]
mod tests;
