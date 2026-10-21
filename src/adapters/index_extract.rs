//! Compatible façade for source indexing extraction.
mod csharp;
mod decode;
mod orchestration;
mod preview;
mod razor;

pub use orchestration::{extract, language};
pub(in crate::adapters) use preview::preview;

const CONTRACT: harness_domain::Failure = (
    "EXTRACTION_CONTRACT",
    "Invalid native CodeGraph extraction buffers",
);

#[cfg(test)]
mod tests;
