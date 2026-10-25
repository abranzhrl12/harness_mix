//! Bundled skill synchronization for Harness projects.
//!
//! The public surface is the sync facade plus [`project_support`] for the
//! CLI's project initialization workflow. Implementation modules remain private.

mod catalog;
mod lock;
mod manifest;
mod preferences;
pub mod project_support;
mod report;
mod storage;
mod sync;

pub use preferences::Preferences;
pub use report::{Action, Report};
pub use sync::{Options, run, sync_on_startup};
