//! Bundled skill synchronization for Harness projects.
//!
//! The public surface includes sync, project support and the reversible
//! project installation lifecycle. Storage implementation remains private.

mod catalog;
mod lock;
mod manifest;
mod preferences;
pub mod project_support;
mod report;
mod storage;
mod sync;
pub mod installation;

pub use preferences::Preferences;
pub use report::{Action, Report};
pub use sync::{Options, run, sync_on_startup};
