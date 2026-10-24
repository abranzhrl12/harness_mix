use std::{ffi::OsString, path::PathBuf};

use crate::{Command, Root};

/// Input boundary used by the CLI use case.
pub trait ArgumentParser {
    fn needs_workspace_root(&self, args: &[OsString]) -> bool;
    fn parse(&self, args: Vec<OsString>) -> Result<Command, String>;
}

/// Workspace discovery boundary used by commands that need a root.
pub trait WorkspaceLocator {
    fn detect_workspace(&self) -> Result<PathBuf, String>;
    fn current_dir(&self) -> Result<PathBuf, String>;
}

/// Coordinates argument normalization, parsing, and root resolution.
pub struct CliApplication<P, W> {
    parser: P,
    workspace: W,
}

impl<P, W> CliApplication<P, W>
where
    P: ArgumentParser,
    W: WorkspaceLocator,
{
    pub fn new(parser: P, workspace: W) -> Self {
        Self { parser, workspace }
    }

    pub fn prepare(&self, args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
        let mut args: Vec<_> = args.into_iter().collect();
        if self.parser.needs_workspace_root(&args) && args.first().is_none_or(|arg| arg != "skills")
        {
            let root = self.workspace.detect_workspace()?;
            args.splice(0..0, [OsString::from("--root"), root.into_os_string()]);
        }
        self.parser.parse(args)
    }

    pub fn resolve_root(&self, root: Root) -> Result<PathBuf, String> {
        match root {
            Root::Explicit(path) => Ok(path),
            Root::Detect => self.workspace.detect_workspace(),
            Root::DetectOrCurrent => Ok(self
                .workspace
                .detect_workspace()
                .or_else(|_| self.workspace.current_dir())
                .unwrap_or_else(|_| PathBuf::from("."))),
        }
    }
}

#[cfg(test)]
mod tests;
