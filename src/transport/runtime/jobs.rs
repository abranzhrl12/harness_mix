use harness_application::ports::{JobEvent, JobEventFuture, JobEventStream, JobFuture, JobService};
use harness_domain::tasks::TaskArgs;
use serde_json::{Value, json};

pub(super) struct MultiJobService {
    pub(super) projects: Vec<crate::adapters::workspace_set::WorkspaceProject>,
    pub(super) managers: Vec<crate::adapters::tasks::TaskManager>,
}

impl JobService for MultiJobService {
    fn handle(&self, args: TaskArgs) -> JobFuture<'_> {
        Box::pin(async move {
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
                    per_project.push(json!({
                        "project": self.projects[index].name,
                        "data": result
                    }));
                }
                return Ok(json!({ "projects": per_project }));
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
