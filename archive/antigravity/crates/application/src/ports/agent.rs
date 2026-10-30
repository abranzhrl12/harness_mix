use harness_domain::agent::AgentProvider;
use std::path::{Path, PathBuf};

/// Launches an installed provider in the selected workspace.
pub trait AgentLauncher {
    fn available(&self, provider: AgentProvider) -> bool;
    fn executable(&self, provider: AgentProvider) -> Option<PathBuf>;
    fn launch(&self, provider: AgentProvider, workspace: &Path) -> Result<i32, String>;
}
