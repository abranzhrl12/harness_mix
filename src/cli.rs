//! Executable host adapters for the reusable CLI crate.

use std::path::{Path, PathBuf};

use agent_harness::adapters::{
    agent::AntigravityLauncher,
    database_setup,
    index_store,
    index_sync,
};
use harness_application::{
    agent::{ActiveAgent, AgentConfiguration},
};
use harness_cli::commands::{
    AgentHost, DatabaseHost, ExecutableHost, IndexHost, IndexOperation, ServerHost,
};
use harness_domain::{agent::{ActiveAgentRequest, AgentProvider}, execution::OperationContext, Failure};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default)]
pub struct AgentHarnessHost;

impl DatabaseHost for AgentHarnessHost {
    fn initialize_database(&self, root: &Path) -> Result<Value, String> {
        database_setup::initialize(root).map_err(|(code, message)| format!("{code}: {message}"))
    }

}

impl IndexHost for AgentHarnessHost {
    fn index(
        &self,
        root: &Path,
        operation: IndexOperation,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        match operation {
            IndexOperation::Initialize => index_store::initialize(root, context),
            IndexOperation::SyncWorkspace => index_store::sync_workspace_all(root, context),
            IndexOperation::Rebuild => index_store::rebuild_workspace(root, context),
            IndexOperation::Plan => index_sync::plan_workspace(root, context),
            IndexOperation::SyncFile { relative, tsconfig, force } => match tsconfig {
                Some(config) => index_store::sync_file_with_tsconfig(root, &relative, &config, context),
                None if force => index_store::sync_file_forced(root, &relative, context),
                None => index_store::sync_file(root, &relative, context),
            },
        }
    }

    fn take_sync_diagnostic(&self, code: &str, message: &str) -> Option<Value> {
        index_store::take_sync_diagnostic(code, message)
    }

}

impl AgentHost for AgentHarnessHost {
    fn agent_executable(&self) -> Option<PathBuf> {
        let statuses = AgentConfiguration::list(&AntigravityLauncher);
        statuses
            .iter()
            .find(|status| status.provider == AgentProvider::Antigravity)
            .and_then(|status| status.executable.clone())
    }

    fn launch_agent(&self, root: &Path, provider: &str) -> Result<i32, String> {
        let provider = match provider {
            "antigravity" => AgentProvider::Antigravity,
            _ => return Err("proveedor no soportado".into()),
        };
        ActiveAgent::execute(
            ActiveAgentRequest { provider, workspace: root.to_path_buf() },
            &AntigravityLauncher,
        )
        .map_err(|error| error.to_string())
    }

}

impl ExecutableHost for AgentHarnessHost {
    fn current_executable(&self) -> Result<PathBuf, String> {
        std::env::current_exe().map_err(|error| error.to_string())
    }
}

impl ServerHost for AgentHarnessHost {
    async fn serve(
        &self,
        root: Option<&Path>,
        workspace_config: Option<&Path>,
        persist_logs: bool,
    ) -> Result<(), String> {
        match (root, workspace_config) {
            (Some(root), None) => agent_harness::transport::serve_with_logging(root, persist_logs).await,
            (None, Some(config)) => agent_harness::transport::serve_with_workspace_config(config, persist_logs).await,
            _ => Err("exactly one serving configuration must be provided".into()),
        }
        .map_err(|error| error.to_string())
    }
}

pub async fn run(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<(), Box<dyn std::error::Error>> {
    harness_cli::commands::run(args, &AgentHarnessHost, env!("CARGO_PKG_VERSION")).await
}
