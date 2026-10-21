//! Native composition and lifecycle for the stdio server.
use super::{McpHarness, input};
use crate::adapters;
use rmcp::ServiceExt;
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

pub async fn serve(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    serve_with_logging(root, false).await
}
pub async fn serve_with_logging(
    root: &Path,
    persist_logs: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let profile_name = match std::env::var("HARNESS_TOOL_PROFILE") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => "full".to_owned(),
        Err(_) => return Err("HARNESS_TOOL_PROFILE must be valid UTF-8".into()),
    };
    let profile = harness_domain::tool_contract::ToolProfile::parse(&profile_name)
        .map_err(|(_, message)| message)?;
    let watch_setting = match std::env::var("HARNESS_INDEX_WATCH").as_deref() {
        Ok("1") => Some(true),
        Ok("0") => Some(false),
        Err(std::env::VarError::NotPresent) => None,
        _ => return Err("HARNESS_INDEX_WATCH must be 0 or 1".into()),
    };
    let watch_enabled = watch_setting
        .unwrap_or(cfg!(windows) && profile == harness_domain::tool_contract::ToolProfile::Full);
    if watch_enabled && profile != harness_domain::tool_contract::ToolProfile::Full {
        return Err("HARNESS_INDEX_WATCH=1 requires the full tool profile".into());
    }
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
        match crate::adapters::index_watch::IndexWatch::start(root) {
            Ok(watch) => {
                let status = watch.status.clone();
                (Some(watch), Some(status))
            }
            Err((code, message)) if watch_setting == Some(true) => {
                return Err(format!("{code}: {message}").into());
            }
            Err((code, _)) => (
                None,
                Some(Arc::new(std::sync::Mutex::new(serde_json::json!({
                    "enabled":false,"state":"inactive","defaultEnabled":true,
                    "lastError":code,"reason":"Existing index unavailable or automatic writer could not start; server remains available"
                })))),
            ),
        }
    } else {
        (None, None)
    };
    // Retain the owner for the entire MCP lifetime, including error cleanup.
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
    let service = McpHarness {
        application: adapters::application_with_options(
            root,
            adapters::NativeApplicationOptions {
                persist_logs,
                tasks: tasks.clone(),
                index_watch: watch_status,
            },
        )?
        .with_tool_profile(profile),
        tasks: tasks.clone(),
        task_notifications: Arc::new(AtomicBool::new(true)),
        task_forwarding: Arc::new(AtomicBool::new(false)),
    }
    .serve((
        input::BoundedLines::new(tokio::io::stdin(), input::MAX_MESSAGE_BYTES),
        tokio::io::stdout(),
    ))
    .await?;
    let result = service.waiting().await;
    if let Some(tasks) = tasks {
        tasks.shutdown();
    }
    result?;
    Ok(())
}
