//! Compatible façade for source indexing extraction.
pub(in crate::adapters) mod angular;
mod csharp;
mod decode;
pub(in crate::adapters) mod framework;
mod orchestration;
mod preview;
mod razor;
mod scss;
mod vue;
pub(in crate::adapters) mod vue_router;
pub use orchestration::{extract, language};
pub(in crate::adapters) use preview::preview;

thread_local! {
    static DIAGNOSTIC: std::cell::RefCell<Option<serde_json::Value>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn take_diagnostic() -> Option<serde_json::Value> {
    DIAGNOSTIC.with(|slot| slot.borrow_mut().take())
}

pub(super) fn set_diagnostic(value: serde_json::Value) {
    DIAGNOSTIC.with(|slot| *slot.borrow_mut() = Some(value));
}

const CONTRACT: harness_domain::Failure = (
    "EXTRACTION_CONTRACT",
    "Invalid native CodeGraph extraction buffers",
);

#[cfg(test)]
mod tests;
