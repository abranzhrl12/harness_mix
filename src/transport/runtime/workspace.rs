use super::{coordinator, jobs::MultiJobService};
use crate::adapters;
use harness_application::ports::JobService;
use harness_config::RuntimeConfig;
use harness_mcp::{BoundedLines, MAX_MESSAGE_BYTES, McpServer};
use rmcp::ServiceExt;
use std::{path::Path, sync::Arc};

pub async fn serve_with_workspace_config(
    manifest: &Path,
    persist_logs: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let projects = crate::adapters::workspace_set::WorkspaceProject::from_manifest(manifest)?;
    let config = RuntimeConfig::from_env(cfg!(windows))?;
    let profile = config.profile;
    let watch_setting = config.index_watch_setting;
    let watch_enabled = config.index_watch_enabled;

    #[cfg(windows)]
    let (index_watches, watch_statuses): (Vec<_>, Vec<_>) = if watch_enabled {
        let mut watches = Vec::new();
        let mut statuses = Vec::with_capacity(projects.len());
        for project in &projects {
            if watch_setting.is_none() && !project.root.join(".codegraph/codegraph.db").exists() {
                statuses.push(Some(Arc::new(std::sync::Mutex::new(serde_json::json!({
                    "enabled":false,"state":"inactive","defaultEnabled":true,
                    "lastError":null,"reason":"index_missing"
                })))));
                continue;
            }
            match crate::adapters::index_watch::IndexWatch::start(&project.root) {
                Ok(watch) => {
                    statuses.push(Some(watch.status.clone()));
                    watches.push(watch);
                }
                Err((code, message)) if watch_setting == Some(true) => {
                    return Err(format!("{code}: {message}").into());
                }
                Err((code, _)) => statuses.push(Some(Arc::new(std::sync::Mutex::new(
                    serde_json::json!({
                        "enabled":false,"state":"inactive","lastError":code,
                        "reason":"Existing index unavailable or automatic writer could not start; server remains available"
                    }),
                )))),
            }
        }
        (watches, statuses)
    } else {
        (Vec::new(), vec![None; projects.len()])
    };

    #[cfg(not(windows))]
    let (index_watches, watch_statuses): (
        Vec<()>,
        Vec<Option<Arc<std::sync::Mutex<serde_json::Value>>>>,
    ) = {
        if watch_enabled {
            return Err("Automatic index publication currently requires Windows".into());
        }
        (Vec::new(), vec![None; projects.len()])
    };
    // Retain every watcher until the lifecycle coordinator has drained jobs.
    let _index_watches = index_watches;

    let mut task_managers = Vec::new();
    if profile.allows("harness_jobs") {
        for project in &projects {
            match crate::adapters::tasks::TaskManager::new(&project.root) {
                Ok(manager) => task_managers.push(manager),
                Err(error) => {
                    return Err(coordinator::startup_error(
                        error,
                        coordinator::drain_managers(&task_managers),
                    )
                    .await);
                }
            }
        }
    }

    let tasks = (!task_managers.is_empty()).then(|| {
        Arc::new(MultiJobService {
            projects: projects.clone(),
            managers: task_managers.clone(),
        }) as Arc<dyn JobService>
    });
    let application = match adapters::application_with_projects(
        &projects,
        persist_logs,
        &task_managers,
        &watch_statuses,
    ) {
        Ok(application) => application.with_tool_profile(profile),
        Err(error) => {
            return Err(coordinator::startup_error(
                error,
                coordinator::drain_managers(&task_managers),
            )
            .await);
        }
    };

    let started = McpServer::new(application, tasks)
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
        coordinator::drain_managers(&task_managers),
    )
    .await
}
