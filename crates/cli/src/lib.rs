//! Command-line interfaces with injected native host capabilities.

mod adapters;
mod application;
mod composition;
mod domain;
mod infrastructure;
mod index_progress;

pub mod commands;
pub mod terminal;

pub use adapters::input::{needs_workspace_root, parse};
pub use application::{ArgumentParser, CliApplication, WorkspaceLocator};
pub use composition::{prepare_command, resolve_root};
pub use domain::{Command, Root};
