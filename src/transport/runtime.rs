//! Native composition and lifecycle for the stdio server.
use super::{McpHarness, input};
use crate::adapters;
use harness_application::ports::{JobEvent, JobEventFuture, JobEventStream, JobFuture, JobService};
use rmcp::ServiceExt;
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

async fn drain_tasks(tasks: Option<&crate::adapters::tasks::TaskManager>) -> Result<(), String> {
    if let Some(tasks) = tasks {
        if !tasks.shutdown_and_wait(Duration::from_secs(10)).await {
            return Err(
                "Background job workers did not drain within the 10-second shutdown deadline"
                    .into(),
            );
        }
    }
    Ok(())
}

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
                ..Default::default()
            },
        )?
        .with_tool_profile(profile),
        tasks: tasks
            .clone()
            .map(|manager| Arc::new(manager) as Arc<dyn JobService>),
        task_notifications: Arc::new(AtomicBool::new(true)),
        task_forwarding: Arc::new(AtomicBool::new(false)),
    }
    .serve((
        input::BoundedLines::new(tokio::io::stdin(), input::MAX_MESSAGE_BYTES),
        tokio::io::stdout(),
    ))
    .await;
    let service = match service {
        Ok(service) => service,
        Err(error) => {
            let drain = drain_tasks(tasks.as_ref()).await;
            return match drain {
                Ok(()) => Err(error.into()),
                Err(drain_error) => {
                    Err(std::io::Error::other(format!("{error}; {drain_error}")).into())
                }
            };
        }
    };
    let result = service.waiting().await;
    drain_tasks(tasks.as_ref())
        .await
        .map_err(std::io::Error::other)?;
    result?;
    Ok(())
}

pub async fn serve_with_workspace_config(
    manifest: &Path,
    persist_logs: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let projects = crate::adapters::workspace_set::WorkspaceProject::from_manifest(manifest)?;
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
                Ok(watch) => { statuses.push(Some(watch.status.clone())); watches.push(watch); }
                Err((code, message)) if watch_setting == Some(true) => return Err(format!("{code}: {message}").into()),
                Err((code, _)) => statuses.push(Some(Arc::new(std::sync::Mutex::new(serde_json::json!({
                    "enabled":false,"state":"inactive","lastError":code,
                    "reason":"Existing index unavailable or automatic writer could not start; server remains available"
                }))))),
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
    let _index_watches = index_watches;
    let task_managers = if profile.allows("harness_jobs") {
        projects
            .iter()
            .map(|project| crate::adapters::tasks::TaskManager::new(&project.root))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let tasks = (!task_managers.is_empty()).then(|| {
        Arc::new(MultiJobService {
            projects: projects.clone(),
            managers: task_managers.clone(),
        }) as Arc<dyn JobService>
    });
    let application = adapters::application_with_projects(
        &projects,
        persist_logs,
        &task_managers,
        &watch_statuses,
    )?
    .with_tool_profile(profile);
    let service = McpHarness {
        application,
        tasks,
        task_notifications: Arc::new(AtomicBool::new(true)),
        task_forwarding: Arc::new(AtomicBool::new(false)),
    }
    .serve((
        input::BoundedLines::new(tokio::io::stdin(), input::MAX_MESSAGE_BYTES),
        tokio::io::stdout(),
    ))
    .await;
    let service = match service {
        Ok(service) => service,
        Err(error) => {
            let drain = drain_managers(&task_managers).await;
            return match drain {
                Ok(()) => Err(error.into()),
                Err(drain_error) => {
                    Err(std::io::Error::other(format!("{error}; {drain_error}")).into())
                }
            };
        }
    };
    let result = service.waiting().await;
    drain_managers(&task_managers)
        .await
        .map_err(std::io::Error::other)?;
    result?;
    Ok(())
}

async fn drain_managers(managers: &[crate::adapters::tasks::TaskManager]) -> Result<(), String> {
    for manager in managers {
        if !manager.shutdown_and_wait(Duration::from_secs(10)).await {
            return Err(
                "Background job workers did not drain within the 10-second shutdown deadline"
                    .into(),
            );
        }
    }
    Ok(())
}

struct MultiJobService {
    projects: Vec<crate::adapters::workspace_set::WorkspaceProject>,
    managers: Vec<crate::adapters::tasks::TaskManager>,
}

impl JobService for MultiJobService {
    fn handle(&self, args: harness_domain::tasks::TaskArgs) -> JobFuture<'_> {
        Box::pin(async move {
            use harness_domain::tasks::TaskArgs;
            use serde_json::{Value, json};
            if args.action == "start" {
                let mut value = serde_json::to_value(&args)
                    .map_err(|_| ("ARGUMENT", "Invalid task arguments"))?;
                let (index, project, relative) = match args
                    .verify
                    .as_ref()
                    .and_then(|verify| verify.scope.as_deref())
                {
                    Some(scope) => {
                        let (project, relative) =
                            crate::adapters::workspace_set::WorkspaceProject::resolve(
                                &self.projects,
                                scope,
                            )
                            .map_err(|_| {
                                (
                                    "ARGUMENT",
                                    "Workspace job path must identify one registered project",
                                )
                            })?;
                        let index = self
                            .projects
                            .iter()
                            .position(|candidate| candidate.name == project.name)
                            .ok_or(("INTERNAL", "Resolved project disappeared"))?;
                        (index, project, relative)
                    }
                    None if self.projects.len() == 1 => (0, &self.projects[0], ".".to_owned()),
                    None => {
                        return Err((
                            "ARGUMENT",
                            "Workspace job requires verify.scope as project:/path",
                        ));
                    }
                };
                value["verify"]["scope"] = Value::String(relative);
                let mapped: TaskArgs = serde_json::from_value(value)
                    .map_err(|_| ("ARGUMENT", "Cannot normalize workspace job path"))?;
                let mut result = self.managers[index].handle(mapped).await?;
                result["project"] = json!(project.name);
                return Ok(result);
            }
            if matches!(args.action.as_str(), "list" | "events") {
                let mut per_project = Vec::new();
                for (index, manager) in self.managers.iter().enumerate() {
                    let value = serde_json::to_value(&args)
                        .map_err(|_| ("ARGUMENT", "Invalid task arguments"))?;
                    let mapped: TaskArgs = serde_json::from_value(value)
                        .map_err(|_| ("ARGUMENT", "Invalid task arguments"))?;
                    let mut result = manager.handle(mapped).await?;
                    let key = if args.action == "list" {
                        "tasks"
                    } else {
                        "events"
                    };
                    if let Some(items) = result[key].as_array_mut() {
                        for item in items {
                            item["project"] = json!(self.projects[index].name);
                        }
                    }
                    per_project.push(json!({"project":self.projects[index].name,"data":result}));
                }
                return Ok(json!({"projects":per_project}));
            }
            let mut last_error = ("TASK_NOT_FOUND", "Task not found or expired");
            for (index, manager) in self.managers.iter().enumerate() {
                let value = serde_json::to_value(&args)
                    .map_err(|_| ("ARGUMENT", "Invalid task arguments"))?;
                let mapped: TaskArgs = serde_json::from_value(value)
                    .map_err(|_| ("ARGUMENT", "Invalid task arguments"))?;
                match manager.handle(mapped).await {
                    Ok(mut result) => {
                        result["project"] = json!(self.projects[index].name);
                        crate::adapters::workspace_set::qualify_paths(
                            &mut result,
                            &self.projects[index].name,
                            true,
                        );
                        return Ok(result);
                    }
                    Err(error) => last_error = error,
                }
            }
            Err(last_error)
        })
    }
    fn subscribe(&self) -> Box<dyn JobEventStream> {
        Box::new(MultiTaskEvents {
            streams: self.managers.iter().map(JobService::subscribe).collect(),
        })
    }
}

struct MultiTaskEvents {
    streams: Vec<Box<dyn JobEventStream>>,
}
impl JobEventStream for MultiTaskEvents {
    fn recv(&mut self) -> JobEventFuture<'_> {
        Box::pin(async move {
            if self.streams.is_empty() {
                return JobEvent::Closed;
            }
            let futures = self
                .streams
                .iter_mut()
                .map(|stream| stream.recv())
                .collect::<Vec<_>>();
            let (event, _, remaining) = futures_util::future::select_all(futures).await;
            drop(remaining);
            event
        })
    }
}
