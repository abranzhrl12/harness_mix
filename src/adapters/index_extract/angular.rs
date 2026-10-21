//! Angular metadata enrichment and generic HTML structure extraction.
//! The metadata adapter never publishes template expressions as semantic refs.

pub(in crate::adapters) mod di;
mod metadata;
mod template;

pub(super) use template::html;

pub(super) fn enrich(
    path: &str,
    source: &str,
    graph: &mut serde_json::Value,
    context: &harness_domain::execution::OperationContext,
) -> Result<(), harness_domain::Failure> {
    metadata::enrich(path, source, graph, context)?;
    di::enrich(path, source, graph, context)
}
