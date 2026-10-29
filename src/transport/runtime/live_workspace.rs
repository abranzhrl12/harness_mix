//! Lazy, transactional workspace reload. No polling thread or idle filesystem work.
use super::{coordinator, jobs::MultiJobService};
use crate::adapters::{
    self, NativeApplication, NativeApplicationOptions, NativeWorkspace,
    tasks::TaskManager,
    workspace_set::{MultiNativeWorkspace, WorkspaceProject},
};
use harness_application::{
    Application,
    ports::{JobEvent, JobEventFuture, JobEventStream, JobFuture, JobService},
};
use harness_config::RuntimeConfig;
use harness_domain::tasks::{TaskAction, TaskArgs};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const CHECK_INTERVAL: Duration = Duration::from_millis(250);
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

struct Resource {
    project: WorkspaceProject,
    workspace: Arc<NativeWorkspace>,
    manager: Option<TaskManager>,
    watch_status: Option<Arc<Mutex<Value>>>,
    #[cfg(windows)]
    watcher: Option<adapters::index_watch::supervisor::SupervisedWatch>,
}

impl Drop for Resource {
    fn drop(&mut self) {
        if let Some(manager) = &self.manager {
            manager.shutdown();
        }
    }
}

struct State {
    application: Option<NativeApplication>,
    anchor: Option<Arc<NativeWorkspace>>,
    resources: BTreeMap<PathBuf, Resource>,
    projects: Vec<WorkspaceProject>,
    last_check: Option<Instant>,
    attempted: Option<Vec<u8>>,
    retry_after: Option<Instant>,
    generation: u64,
    forwarders: Vec<tokio::task::AbortHandle>,
}

pub(super) struct LiveWorkspace {
    manifest: PathBuf,
    config: RuntimeConfig,
    persist_logs: bool,
    status: Arc<Mutex<Value>>,
    state: Mutex<State>,
    events: tokio::sync::broadcast::Sender<Value>,
}

impl Drop for LiveWorkspace {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(|e| e.into_inner());
        for forwarder in &state.forwarders {
            forwarder.abort();
        }
    }
}

impl LiveWorkspace {
    pub(super) fn new(
        manifest: &Path,
        config: RuntimeConfig,
        persist_logs: bool,
    ) -> std::io::Result<Arc<Self>> {
        let manifest = manifest.canonicalize()?;
        let (events, _) = tokio::sync::broadcast::channel(256);
        let live = Arc::new(Self {
            manifest,
            config,
            persist_logs,
            events,
            status: Arc::new(Mutex::new(
                json!({"enabled":true,"checkIntervalMs":250,"generation":0,"lastError":null}),
            )),
            state: Mutex::new(State {
                application: None,
                anchor: None,
                resources: BTreeMap::new(),
                projects: Vec::new(),
                last_check: None,
                attempted: None,
                retry_after: None,
                generation: 0,
                forwarders: Vec::new(),
            }),
        });
        {
            let mut state = live.state.lock().unwrap_or_else(|e| e.into_inner());
            live.reload(&mut state)?;
        }
        Ok(live)
    }

    fn bytes(&self) -> std::io::Result<Vec<u8>> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(&self.manifest)?
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(std::io::Error::other("workspace manifest exceeds 1 MiB"));
        }
        Ok(bytes)
    }

    fn reload(&self, state: &mut State) -> std::io::Result<()> {
        state.last_check = Some(Instant::now());
        let bytes = self.bytes()?;
        if state.attempted.as_ref() == Some(&bytes)
            && state.retry_after.is_none_or(|at| Instant::now() < at)
        {
            return Ok(());
        }
        // Parse the exact bytes compared above, never a second concurrent file read.
        state.attempted = Some(bytes.clone());
        let projects = WorkspaceProject::from_manifest_bytes(&self.manifest, &bytes)?;
        let mut pending = Vec::new();
        let mut initial_sessions = None;
        for project in &projects {
            if state.resources.contains_key(&project.root)
                || pending.iter().any(|(root, _)| root == &project.root)
            {
                continue;
            }
            sync_skills(project);
            let manager = self
                .config
                .profile
                .allows("harness_jobs")
                .then(|| TaskManager::new(&project.root))
                .transpose()?;
            let (watcher, watch_status) = self.watch(&project.root, None)?;
            let (workspace, sessions) = adapters::workspace_dependencies(
                &project.root,
                NativeApplicationOptions {
                    persist_logs: self.persist_logs,
                    tasks: manager.clone(),
                    index_watch: watch_status.clone(),
                    ..Default::default()
                },
            )?;
            if initial_sessions.is_none() {
                initial_sessions = Some(sessions);
            }
            pending.push((
                project.root.clone(),
                Resource {
                    project: project.clone(),
                    workspace: Arc::new(workspace),
                    manager,
                    watch_status,
                    #[cfg(windows)]
                    watcher,
                },
            ));
            #[cfg(not(windows))]
            let _ = watcher;
        }
        #[cfg(windows)]
        let mut resumed = Vec::new();
        for project in &projects {
            if state.projects.iter().any(|p| p.root == project.root) {
                continue;
            }
            if let Some(resource) = state.resources.get(&project.root) {
                sync_skills(project);
                #[cfg(windows)]
                resumed.push((
                    project.root.clone(),
                    self.watch(&project.root, resource.watch_status.clone())?.0,
                ));
                #[cfg(not(windows))]
                let _ = resource;
            }
        }
        // All new projects are ready before the routing table is published.
        for (root, resource) in pending {
            if let Some(manager) = &resource.manager {
                let mut receiver = manager.subscribe();
                let sender = self.events.clone();
                let alias = resource.project.name.clone();
                let forwarder = tokio::spawn(async move {
                    loop {
                        match receiver.recv().await {
                            Ok(mut event) => {
                                event["project"] = json!(alias);
                                let _ = sender.send(event);
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                let _ = sender.send(json!({"kind":"events_lagged","skipped":skipped,"project":alias}));
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
                state.forwarders.push(forwarder.abort_handle());
            }
            state.resources.insert(root, resource);
        }
        #[cfg(windows)]
        for (root, watcher) in resumed {
            state
                .resources
                .get_mut(&root)
                .expect("cached project")
                .watcher = watcher;
        }
        let workspaces = projects
            .iter()
            .map(|p| (p.clone(), state.resources[&p.root].workspace.clone()))
            .collect::<Vec<_>>();
        let anchor = state
            .anchor
            .get_or_insert_with(|| workspaces[0].1.clone())
            .clone();
        let workspace = MultiNativeWorkspace {
            projects: workspaces,
            workspace_mode: true,
            anchor: Some(anchor),
            reload_status: Some(self.status.clone()),
        };
        state.application = Some(match &state.application {
            Some(application) => application.with_workspace(workspace),
            None => Application::new(
                workspace,
                initial_sessions.expect("initial project sessions"),
            )
            .with_tool_profile(self.config.profile),
        });
        #[cfg(windows)]
        for (root, resource) in &mut state.resources {
            if !projects.iter().any(|p| &p.root == root) {
                resource.watcher.take();
                if let Some(status) = &resource.watch_status {
                    *status.lock().unwrap_or_else(|e| e.into_inner()) =
                        json!({"enabled":false,"state":"inactive","reason":"project_removed"});
                }
            }
        }
        state.projects = projects;
        state.retry_after = None;
        state.generation += 1;
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = json!({"enabled":true,"checkIntervalMs":250,"generation":state.generation,"lastError":null});
        Ok(())
    }

    pub(super) fn application(&self) -> NativeApplication {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .last_check
            .is_none_or(|at| at.elapsed() >= CHECK_INTERVAL)
        {
            if let Err(error) = self.reload(&mut state) {
                state.retry_after = Some(Instant::now() + Duration::from_secs(2));
                let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
                let message = error.to_string();
                if status["lastError"].as_str() != Some(&message) {
                    eprintln!(
                        "harness workspace reload rejected: {message}; keeping last valid configuration"
                    );
                }
                status["lastError"] = json!(message);
            }
        }
        state
            .application
            .as_ref()
            .expect("validated initial application")
            .clone()
    }

    fn jobs(&self, active_only: bool) -> MultiJobService {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let pairs = if active_only {
            state
                .projects
                .iter()
                .filter_map(|p| {
                    state.resources[&p.root]
                        .manager
                        .clone()
                        .map(|m| (p.clone(), m))
                })
                .collect::<Vec<_>>()
        } else {
            state
                .resources
                .values()
                .filter_map(|r| r.manager.clone().map(|m| (r.project.clone(), m)))
                .collect::<Vec<_>>()
        };
        let (projects, managers) = pairs.into_iter().unzip();
        MultiJobService { projects, managers }
    }

    pub(super) async fn drain(&self) -> Result<(), String> {
        let jobs = self.jobs(false);
        coordinator::drain_managers(&jobs.managers).await
    }

    #[cfg(windows)]
    fn watch(
        &self,
        root: &Path,
        previous_status: Option<Arc<Mutex<Value>>>,
    ) -> std::io::Result<(
        Option<adapters::index_watch::supervisor::SupervisedWatch>,
        Option<Arc<Mutex<Value>>>,
    )> {
        if !self.config.index_watch_enabled {
            return Ok((None, None));
        }
        let status = previous_status.unwrap_or_else(|| Arc::new(Mutex::new(json!({}))));
        if self.config.index_watch_setting.is_none()
            && !root.join(".codegraph/codegraph.db").exists()
        {
            *status.lock().unwrap_or_else(|e| e.into_inner()) =
                json!({"enabled":false,"state":"inactive","reason":"index_missing"});
            return Ok((None, Some(status)));
        }
        match adapters::index_watch::supervisor::SupervisedWatch::start_with_status(
            root,
            status.clone(),
        ) {
            Ok(watch) => {
                let status = watch.status.clone();
                Ok((Some(watch), Some(status)))
            }
            Err((code, message)) if self.config.index_watch_setting == Some(true) => {
                Err(std::io::Error::other(format!("{code}: {message}")))
            }
            Err((code, message)) => {
                *status.lock().unwrap_or_else(|e| e.into_inner()) =
                    json!({"enabled":false,"state":"inactive","lastError":code,"reason":message});
                Ok((None, Some(status)))
            }
        }
    }

    #[cfg(not(windows))]
    fn watch(
        &self,
        _: &Path,
        _: Option<Arc<Mutex<Value>>>,
    ) -> std::io::Result<(Option<()>, Option<Arc<Mutex<Value>>>)> {
        Ok((None, None))
    }
}

fn sync_skills(project: &WorkspaceProject) {
    match harness_skills::project_support::initialize_workspace_project(&project.root) {
        Ok(Some(report)) if report.conflicts > 0 || report.errors > 0 => eprintln!(
            "harness skills auto-sync [{}]: {}",
            project.name,
            serde_json::to_string(&report).unwrap_or_default()
        ),
        Ok(_) => {}
        Err(error) => eprintln!(
            "harness skills auto-sync [{}] skipped: {error}",
            project.name
        ),
    }
}

pub(super) struct LiveJobs(pub(super) Arc<LiveWorkspace>);
impl JobService for LiveJobs {
    fn handle(&self, args: TaskArgs) -> JobFuture<'_> {
        Box::pin(async move {
            let source = self.0.clone();
            tokio::task::spawn_blocking(move || source.application())
                .await
                .map_err(|_| ("INTERNAL", "Workspace snapshot provider failed"))?;
            self.0
                .jobs(args.action == TaskAction::Start)
                .handle(args)
                .await
        })
    }
    fn subscribe(&self) -> Box<dyn JobEventStream> {
        Box::new(Events(self.0.events.subscribe()))
    }
}
struct Events(tokio::sync::broadcast::Receiver<Value>);
impl JobEventStream for Events {
    fn recv(&mut self) -> JobEventFuture<'_> {
        Box::pin(async move {
            match self.0.recv().await {
                Ok(value) => JobEvent::Event(value),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => JobEvent::Lagged(n),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => JobEvent::Closed,
            }
        })
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn removed_project_stops_its_watcher_and_reactivation_reuses_status() {
        let temporary = tempfile::tempdir().unwrap();
        let backend = temporary.path().join("backend");
        let frontend = temporary.path().join("frontend");
        std::fs::create_dir(&backend).unwrap();
        std::fs::create_dir(&frontend).unwrap();
        crate::adapters::index_store::initialize(
            &backend,
            &harness_domain::execution::OperationContext::default(),
        )
        .unwrap();
        let manifest = temporary.path().join("workspace.json");
        std::fs::write(&manifest, r#"{"projects":{"backend":"backend"}}"#).unwrap();
        let config = RuntimeConfig::from_values(Ok(None), Ok(None), None, Ok(None), true).unwrap();
        let live = LiveWorkspace::new(&manifest, config, false).unwrap();
        let root = backend.canonicalize().unwrap();
        let status = {
            let state = live.state.lock().unwrap();
            assert!(state.resources[&root].watcher.is_some());
            state.resources[&root].watch_status.clone().unwrap()
        };
        std::fs::write(&manifest, r#"{"projects":{"frontend":"frontend"}}"#).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        live.application();
        assert!(
            live.state.lock().unwrap().resources[&root]
                .watcher
                .is_none()
        );
        assert_eq!(status.lock().unwrap()["reason"], "project_removed");
        std::fs::write(&manifest, r#"{"projects":{"backend":"backend"}}"#).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        live.application();
        {
            let state = live.state.lock().unwrap();
            assert!(state.resources[&root].watcher.is_some());
            assert!(Arc::ptr_eq(
                &status,
                state.resources[&root].watch_status.as_ref().unwrap()
            ));
            assert_eq!(state.projects[0].name, "backend");
        }
        live.drain().await.unwrap();
    }
}
