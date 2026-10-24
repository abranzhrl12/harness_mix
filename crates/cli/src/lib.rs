//! Command-line parsing and preparation, independent from the host command executor.

mod adapters;
mod application;
mod composition;
mod domain;
mod infrastructure;

pub use adapters::input::{needs_workspace_root, parse};
pub use application::{ArgumentParser, CliApplication, WorkspaceLocator};
pub use composition::{prepare_command, resolve_root};
pub use domain::{Command, Root};
