use super::{
    coordinator,
    live_workspace::{LiveJobs, LiveWorkspace},
};
use harness_application::ports::JobService;
use harness_config::RuntimeConfig;
use harness_mcp::{BoundedLines, MAX_MESSAGE_BYTES, McpServer};
use rmcp::ServiceExt;
use std::{path::Path, sync::Arc};

pub async fn serve_with_workspace_config(
    manifest: &Path,
    persist_logs: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = RuntimeConfig::from_env(cfg!(windows))?;
    let live = LiveWorkspace::new(manifest, config, persist_logs)?;
    let source = live.clone();
    let tasks = config
        .profile
        .allows("harness_jobs")
        .then(|| Arc::new(LiveJobs(live.clone())) as Arc<dyn JobService>);
    let started = McpServer::new(live.application(), tasks)
        .with_application_source(move || source.application())
        .with_eager_tools(config.eager_tools_enabled)
        .with_context_presentation(config.context_presentation)
        .serve((
            BoundedLines::new(tokio::io::stdin(), MAX_MESSAGE_BYTES),
            tokio::io::stdout(),
        ))
        .await;
    coordinator::run_server(
        started,
        |service| async move { service.waiting().await.map(|_| ()) },
        live.drain(),
    )
    .await
}
