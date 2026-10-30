use crate::ports::agent::AgentLauncher;
use harness_domain::agent::{ActiveAgentRequest, AgentProvider, AgentStatus};

/// Starts the provider selected by the CLI and returns its exit code.
pub struct ActiveAgent;

impl ActiveAgent {
    pub fn execute(
        request: ActiveAgentRequest,
        launcher: &impl AgentLauncher,
    ) -> Result<i32, String> {
        if !launcher.available(request.provider) {
            return Err(format!(
                "{} CLI is unavailable; install it and make it discoverable on PATH",
                request.provider.as_str()
            ));
        }
        launcher.launch(request.provider, &request.workspace)
    }
}

/// Reports configured providers without starting them or changing their settings.
pub struct AgentConfiguration;

impl AgentConfiguration {
    pub fn list(launcher: &impl AgentLauncher) -> Vec<AgentStatus> {
        AgentProvider::SUPPORTED
            .iter()
            .copied()
            .map(|provider| AgentStatus {
                provider,
                available: launcher.available(provider),
                executable: launcher.executable(provider),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{ActiveAgent, AgentConfiguration};
    use crate::ports::agent::AgentLauncher;
    use harness_domain::agent::{ActiveAgentRequest, AgentProvider};
    use std::path::{Path, PathBuf};

    struct FakeLauncher {
        is_available: bool,
        executable: Option<PathBuf>,
        exit_code: i32,
    }

    impl AgentLauncher for FakeLauncher {
        fn available(&self, _provider: AgentProvider) -> bool {
            self.is_available
        }

        fn executable(&self, _provider: AgentProvider) -> Option<PathBuf> {
            self.executable.clone()
        }

        fn launch(&self, _provider: AgentProvider, _workspace: &Path) -> Result<i32, String> {
            Ok(self.exit_code)
        }
    }

    #[test]
    fn active_agent_returns_provider_exit_code() {
        let launcher = FakeLauncher {
            is_available: true,
            executable: Some(PathBuf::from("/tools/agy")),
            exit_code: 17,
        };

        let result = ActiveAgent::execute(
            ActiveAgentRequest {
                provider: AgentProvider::Antigravity,
                workspace: PathBuf::from("/workspace"),
            },
            &launcher,
        );

        assert_eq!(result, Ok(17));
    }

    #[test]
    fn configuration_lists_only_implemented_providers_and_their_resolution() {
        let launcher = FakeLauncher {
            is_available: true,
            executable: Some(PathBuf::from("/tools/agy")),
            exit_code: 0,
        };

        let status = AgentConfiguration::list(&launcher);

        assert_eq!(status.len(), 1);
        assert_eq!(status[0].provider, AgentProvider::Antigravity);
        assert!(status[0].available);
        assert_eq!(
            status[0].executable.as_deref(),
            Some(Path::new("/tools/agy"))
        );
    }
}
