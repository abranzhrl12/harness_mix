//! Vue Router extraction: literal route tables and component references.
use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

pub(super) mod parse;
mod presentation;
pub(in crate::adapters) mod bindings;

pub(in crate::adapters) use bindings::{named_runtime_import, proves_reference};

const MAX_ROUTES: usize = parse::MAX_ROUTES;

pub(super) fn enrich(
    path: &str,
    source: &str,
    graph: &mut Value,
    context: &OperationContext,
) -> Result<(), Failure> {
    let routes = parse::registered_routes(source, path, context)?;
    if routes.is_empty() {
        return Ok(());
    }
    let language = crate::adapters::index_extract::language(path).unwrap_or("typescript");
    let (mut nodes, mut references) =
        presentation::project(path, source, language, routes, context)?;
    if nodes.len() > MAX_ROUTES || references.len() > MAX_ROUTES * 4 {
        return Err((
            "EXTRACTION_LIMIT",
            "Vue route graph exceeds configured limit",
        ));
    }
    graph
        .get_mut("nodes")
        .and_then(Value::as_array_mut)
        .ok_or(("EXTRACTION_CONTRACT", "Invalid graph nodes"))?
        .append(&mut nodes);
    graph
        .get_mut("unresolvedReferences")
        .and_then(Value::as_array_mut)
        .ok_or(("EXTRACTION_CONTRACT", "Invalid graph references"))?
        .append(&mut references);
    Ok(())
}
