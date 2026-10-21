//! Angular metadata enrichment and generic HTML structure extraction.
//! The metadata adapter never publishes template expressions as semantic refs.

mod metadata;
mod template;

pub(super) use metadata::enrich;
pub(super) use template::html;
