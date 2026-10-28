use super::coordinator;
use crate::adapters;
use harness_application::ports::JobService;
use harness_config::RuntimeConfig;
use harness_mcp::{BoundedLines, MAX_MESSAGE_BYTES, McpServer};
use rmcp::ServiceExt;
use std::{path::Path, sync::Arc};

pub async fn serve(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    serve_with_logging(root, false).await
}

pub async fn serve_with_logging(
    root: &Path,
    persist_logs: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = RuntimeConfig::from_env(cfg!(windows))?;
    let profile = config.profile;
    let watch_setting = config.index_watch_setting;
    let watch_enabled = config.index_watch_enabled;

    #[cfg(windows)]
    let (index_watch, watch_status) = if watch_enabled
        && watch_setting.is_none()
        && matches!(root.join(".codegraph/codegraph.db").try_exists(), Ok(false))
    {
        (
            None,
            Some(Arc::new(std::sync::Mutex::new(serde_json::json!({
                "enabled":false,"state":"inactive","defaultEnabled":true,
                "lastError":null,"reason":"index_missing"
            })))),
        )
    } else if watch_enabled {
        match crate::adapters::index_watch::supervisor::SupervisedWatch::start(root) {
            Ok(watch) => {
                let status = watch.status.clone();
                (Some(watch), Some(status))
            }
            Err((code, message)) if watch_setting == Some(true) => {
                return Err(format!("{code}: {message}").into());
            }
            Err((code, message)) => (
                None,
                Some(Arc::new(std::sync::Mutex::new(serde_json::json!({
                    "enabled":false,"state":"inactive","defaultEnabled":true,
                    "lastError":code,"reason":message
                })))),
            ),
        }
    } else {
        (None, None)
    };

    // Keep the watcher alive through both the service wait and task drain.
    #[cfg(windows)]
    let _index_watch = index_watch;
    #[cfg(not(windows))]
    let watch_status = {
        if watch_enabled {
            return Err("Automatic index publication currently requires Windows".into());
        }
        None
    };

    let tasks = if profile.allows("harness_jobs") {
        Some(crate::adapters::tasks::TaskManager::new(root)?)
    } else {
        None
    };

    let application = match adapters::application_with_options(
        root,
        adapters::NativeApplicationOptions {
            persist_logs,
            tasks: tasks.clone(),
            index_watch: watch_status,
            ..Default::default()
        },
    ) {
        Ok(application) => application.with_tool_profile(profile),
        Err(error) => {
            return Err(coordinator::startup_error(
                error,
                coordinator::drain_task_manager(tasks.as_ref()),
            )
            .await);
        }
    };

    let started = McpServer::new(
        application,
        tasks
            .clone()
            .map(|manager| Arc::new(manager) as Arc<dyn JobService>),
    )
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
        coordinator::drain_task_manager(tasks.as_ref()),
    )
    .await
}
