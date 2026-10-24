use std::{ffi::OsString, path::PathBuf};

use crate::{
    CliApplication, Command, adapters::input::Parser,
    infrastructure::workspace::NativeWorkspaceLocator,
};

type NativeCli = CliApplication<Parser, NativeWorkspaceLocator>;

fn native_cli() -> NativeCli {
    CliApplication::new(Parser, NativeWorkspaceLocator)
}

/// Prepare a command using the native argument parser and workspace discovery.
pub fn prepare_command(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    native_cli().prepare(args)
}

/// Resolve the parsed root using the native workspace discovery policy.
pub fn resolve_root(root: crate::Root) -> Result<PathBuf, String> {
    native_cli().resolve_root(root)
}
