//! Compatible façade for source indexing extraction.
mod angular;
mod csharp;
mod decode;
mod framework;
mod orchestration;
mod preview;
mod razor;
mod vue;

pub use orchestration::{extract, language};
pub(in crate::adapters) use preview::preview;

const CONTRACT: harness_domain::Failure = (
    "EXTRACTION_CONTRACT",
    "Invalid native CodeGraph extraction buffers",
);

#[cfg(test)]
mod tests;
