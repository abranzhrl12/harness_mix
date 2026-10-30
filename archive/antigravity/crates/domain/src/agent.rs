//! Domain contracts for launching a locally configured coding agent.
use std::path::PathBuf;

/// Providers this binary can launch. Add a variant only when its adapter exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentProvider {
    Antigravity,
}

impl AgentProvider {
    pub const SUPPORTED: &'static [Self] = &[Self::Antigravity];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Antigravity => "antigravity",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveAgentRequest {
    pub provider: AgentProvider,
    pub workspace: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentStatus {
    pub provider: AgentProvider,
    pub available: bool,
    pub executable: Option<PathBuf>,
}
