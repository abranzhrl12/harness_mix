//! Projection of parsed Vue route declarations into Harness graph records.
use super::parse::{ComponentRef, RouteEntry};
use crate::adapters::index_extract::framework;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};

const MAX_ROUTES: usize = super::parse::MAX_ROUTES;

fn route_name(prefix: &str, segment: &str) -> String {
    if segment.is_empty() {
        if prefix.is_empty() {
            "/".into()
        } else {
            prefix.into()
        }
    } else if segment.starts_with('/') {
        segment.into()
    } else if prefix.is_empty() || prefix == "/" {
        format!("/{segment}")
    } else {
        format!("{prefix}/{segment}")
    }
}

fn add_reference(
    references: &mut Vec<Value>,
    route_id: &str,
    component: &ComponentRef,
    path: &str,
    language: &str,
    positions: &framework::Positions<'_>,
) -> Result<(), Failure> {
    if references.len() >= MAX_ROUTES * 4 {
        return Err((
            "EXTRACTION_LIMIT",
            "Vue route references exceed configured limit",
        ));
    }
    let (line, column) = positions.position(component.at)?;
    references.push(json!({
        "fromNodeId": route_id,
        "referenceName": component.name,
        "referenceKind": component.kind,
        "line": line,
        "column": column,
        "filePath": path,
        "language": language
    }));
    Ok(())
}

fn emit_routes(
    path: &str,
    language: &str,
    entries: &[RouteEntry],
    prefix: &str,
    positions: &framework::Positions<'_>,
    nodes: &mut Vec<Value>,
    references: &mut Vec<Value>,
    context: &OperationContext,
) -> Result<(), Failure> {
    for entry in entries {
        context.check()?;
        let effective_path = route_name(prefix, &entry.path);
        let id = framework::id("vue_router", path, "route", entry.path_at);
        let (line, column) = positions.position(entry.path_at)?;
        nodes.push(json!({
            "id": id,
            "kind": "route",
            "name": effective_path,
            "qualifiedName": format!("{path}::route:{effective_path}"),
            "filePath": path,
            "language": language,
            "startLine": line,
            "endLine": line,
            "startColumn": column,
            "endColumn": column,
            "signature": "Vue Router route declaration"
        }));
        if let Some(component) = &entry.component {
            add_reference(references, &id, component, path, language, positions)?;
        }
        emit_routes(
            path,
            language,
            &entry.children,
            &effective_path,
            positions,
            nodes,
            references,
            context,
        )?;
    }
    Ok(())
}

pub(super) fn project(
    path: &str,
    source: &str,
    language: &str,
    routes: Vec<Vec<RouteEntry>>,
    context: &OperationContext,
) -> Result<(Vec<Value>, Vec<Value>), Failure> {
    let mut nodes = Vec::new();
    let mut references = Vec::new();
    let positions = framework::Positions::new(source);
    for entries in routes {
        context.check()?;
        emit_routes(
            path,
            language,
            &entries,
            "",
            &positions,
            &mut nodes,
            &mut references,
            context,
        )?;
    }
    Ok((nodes, references))
}
