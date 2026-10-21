//! Bounded native process execution. No shell, Node, taskkill or user-selected executable.
mod capture;
mod environment;
mod executable;

#[cfg(test)]
#[path = "process/tests.rs"]
mod tests;

use capture::run_command_observed;
use environment::{dotnet_environment, git_environment, go_environment, node_environment};
use harness_application::ports::ProcessRunner;
use harness_domain::{
    Failure,
    execution::OperationContext,
    process::{ProcessOutcome, ProcessSpec, Program},
};
use std::path::{Path, PathBuf};

#[cfg(test)]
use capture::run_command;
#[cfg(test)]
use executable::has_visual_studio_web_targets;

pub(in crate::adapters) use executable::executable;

/// Root is fixed at construction, never taken from unvalidated request text.
pub struct NativeProcessRunner {
    root: PathBuf,
}

impl NativeProcessRunner {
    pub fn new(root: &Path) -> Result<Self, Failure> {
        let root = root
            .canonicalize()
            .map_err(|_| ("PATH", "Workspace unavailable"))?;
        if !root.is_dir() {
            return Err(("PATH", "Workspace must be a directory"));
        }
        Ok(Self { root })
    }

    /// Observers must return promptly; process output remains bounded and the
    /// observer receives only the retained prefix of each output chunk.
    pub fn run_observed(
        &self,
        spec: &ProcessSpec,
        context: &OperationContext,
        mut observer: impl FnMut(bool, &[u8]),
    ) -> Result<ProcessOutcome, Failure> {
        context.check()?;
        let mut command = std::process::Command::new(executable(spec.program())?);
        command.args(spec.arguments()).current_dir(&self.root);
        if matches!(spec.program(), Program::Dotnet | Program::Msbuild) {
            dotnet_environment(&mut command);
        }
        if spec.program() == Program::Node {
            node_environment(&mut command);
        }
        if spec.program() == Program::Go {
            // Closed local-toolchain policy, not a sandbox for project/test code.
            go_environment(&mut command);
        }
        if spec.program() == Program::Git {
            git_environment(&mut command);
        }
        run_command_observed(command, spec, context, &mut observer)
    }
}

impl ProcessRunner for NativeProcessRunner {
    fn run(
        &self,
        spec: &ProcessSpec,
        context: &OperationContext,
    ) -> Result<ProcessOutcome, Failure> {
        self.run_observed(spec, context, |_, _| {})
    }
}
