//! Projection of parsed Angular route declarations into Harness graph records.
use super::parse::{ComponentRef, Registration, RouteEntry};
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
            "Angular route references exceed configured limit",
        ));
    }
    let (line, column) = positions.position(component.at)?;
    references.push(json!({
        "fromNodeId":route_id,"referenceName":component.name,
        "referenceKind":component.kind,"line":line,"column":column,
        "filePath":path,"language":language
    }));
    Ok(())
}

fn emit_routes(
    path: &str,
    language: &str,
    registration: Registration,
    entries: &[RouteEntry],
    prefix: &str,
    layouts: &[ComponentRef],
    positions: &framework::Positions<'_>,
    nodes: &mut Vec<Value>,
    references: &mut Vec<Value>,
    context: &OperationContext,
) -> Result<(), Failure> {
    for entry in entries {
        context.check()?;
        let effective_path = route_name(prefix, &entry.path);
        // forChild has no known mount yet. Keep the declaration visibly relative.
        let name = if registration == Registration::Child {
            format!("@forChild{}", effective_path)
        } else {
            effective_path.clone()
        };
        let child_claims_address = entry.children.iter().any(|child| child.path.is_empty());
        let mut child_layouts = layouts.to_vec();
        if let Some(component) = &entry.component {
            if !child_claims_address {
                if nodes.len() >= MAX_ROUTES {
                    return Err(("EXTRACTION_LIMIT", "Angular routes exceed configured limit"));
                }
                let id = framework::id("angular_router", path, "route", entry.path_at);
                let (line, column) = positions.position(entry.path_at)?;
                nodes.push(json!({
                    "id":id,"kind":"route","name":name,
                    "qualifiedName":format!("{path}::route:{name}"),
                    "filePath":path,"language":language,
                    "startLine":line,"endLine":line,"startColumn":column,"endColumn":column,
                    "signature":if registration == Registration::Child {"Angular forChild relative route; mount unresolved"} else {"Angular forRoot/provideRouter route declaration; runtime activation not evaluated"}
                }));
                add_reference(references, &id, component, path, language, positions)?;
                for layout in layouts {
                    add_reference(
                        references,
                        &id,
                        &ComponentRef {
                            kind: "angular_route_layout",
                            ..layout.clone()
                        },
                        path,
                        language,
                        positions,
                    )?;
                }
            }
            child_layouts.push(component.clone());
        }
        emit_routes(
            path,
            language,
            registration,
            &entry.children,
            &effective_path,
            &child_layouts,
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
    routes: Vec<(Vec<RouteEntry>, Registration)>,
    context: &OperationContext,
) -> Result<(Vec<Value>, Vec<Value>), Failure> {
    let mut nodes = Vec::new();
    let mut references = Vec::new();
    let positions = framework::Positions::new(source);
    for (entries, registration) in routes {
        context.check()?;
        // Positions are created from each source in the caller to retain the full
        // file's UTF-16 coordinates, including astral text in preceding markup.
        emit_routes(
            path,
            language,
            registration,
            &entries,
            "",
            &[],
            &positions,
            &mut nodes,
            &mut references,
            context,
        )?;
    }
    Ok((nodes, references))
}
