use harness_application::{
    Application,
    ports::{SessionContext, SessionRepository, workspace},
};
use harness_domain::{
    Failure,
    contracts::{BatchArgs, ReadArgs, Request, SearchArgs},
    execution::OperationContext,
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

struct SchedulerWorkspace {
    first_started: mpsc::SyncSender<()>,
    release_first: Mutex<mpsc::Receiver<()>>,
    third_started: mpsc::SyncSender<()>,
}

impl workspace::WorkspaceMetadata for SchedulerWorkspace {
    fn info(&self) -> Value {
        json!({"root":"memory","languages":[],"manifestScope":"raíz del proyecto","indexed":false})
    }

    fn rules(&self, _: &OperationContext) -> Result<String, Failure> {
        Ok(String::new())
    }

    fn capabilities(&self) -> Value {
        json!({})
    }
}

impl workspace::SourceQueries for SchedulerWorkspace {
    fn read(&self, args: &ReadArgs, _: &OperationContext) -> Result<Value, Failure> {
        match args.path.as_str() {
            "first" => {
                let _ = self.first_started.send(());
                self.release_first
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|_| ("TIMEOUT", "test gate timed out"))?;
            }
            "third" => {
                let _ = self.third_started.send(());
            }
            _ => {}
        }
        Ok(json!({"path":args.path,"text":"memory"}))
    }

    fn search(&self, _: &SearchArgs, _: &OperationContext) -> Result<Value, Failure> {
        Ok(json!({"matches":[]}))
    }
}

impl workspace::SourceEditing for SchedulerWorkspace {}
impl workspace::WorkspaceDiagnostics for SchedulerWorkspace {}
impl workspace::VersionControl for SchedulerWorkspace {}
impl workspace::SqlAccess for SchedulerWorkspace {}
impl workspace::ArtifactQueries for SchedulerWorkspace {}
impl harness_application::ports::browser::BrowserInspection for SchedulerWorkspace {}
impl workspace::SessionPersistence for SchedulerWorkspace {}
impl workspace::LogPersistence for SchedulerWorkspace {}
impl workspace::Verification for SchedulerWorkspace {}

struct TestSessions {
    gate: Arc<Mutex<()>>,
}

impl SessionRepository for TestSessions {
    fn workflow_gate(&mut self, _: &str) -> Result<Arc<Mutex<()>>, Failure> {
        Ok(self.gate.clone())
    }

    fn acquire(&mut self, _: &str) -> Result<(), Failure> {
        Ok(())
    }

    fn release(&mut self, _: &str) {}

    fn open(&mut self, _: Option<&str>) -> Result<SessionContext, Failure> {
        Ok(SessionContext {
            id: "memory".into(),
            restored: false,
            rules_hash: None,
            capabilities_hash: None,
            task_summary: json!({}),
            persistence_warning: None,
        })
    }

    fn update_hashes(&mut self, _: &str, _: String, _: String) -> Result<(), Failure> {
        Ok(())
    }
}

#[test]
fn starts_a_queued_child_when_any_worker_finishes() {
    let (first_started_tx, first_started_rx) = mpsc::sync_channel(1);
    let (release_first_tx, release_first_rx) = mpsc::sync_channel(1);
    let (third_started_tx, third_started_rx) = mpsc::sync_channel(1);
    let app = Application::new(
        SchedulerWorkspace {
            first_started: first_started_tx,
            release_first: Mutex::new(release_first_rx),
            third_started: third_started_tx,
        },
        TestSessions {
            gate: Arc::default(),
        },
    );
    let jobs: Vec<_> = ["first", "second", "third"]
        .into_iter()
        .map(|path| {
            json!({
                "tool":"harness_read",
                "arguments":{"path":path,"start":1,"end":1}
            })
        })
        .collect();
    let args: BatchArgs = serde_json::from_value(json!({
        "maxConcurrent": 2,
        "jobs": jobs
    }))
    .unwrap();
    let worker = std::thread::spawn(move || app.execute(Request::Batch(args)));

    first_started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("first child should start");
    let third_started = third_started_rx
        .recv_timeout(Duration::from_secs(2))
        .is_ok();
    let _ = release_first_tx.send(());
    let result = worker.join().expect("batch coordinator should finish");

    assert!(
        third_started,
        "third child should start while first is still blocked: {result}"
    );
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["results"][2]["index"], 2);
}
