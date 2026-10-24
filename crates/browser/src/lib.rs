//! Browser inspection service with a runtime-specific infrastructure adapter.

mod application;
mod composition;
mod domain;
mod infrastructure;

pub use application::BrowserBackend;
pub use composition::BrowserService;
