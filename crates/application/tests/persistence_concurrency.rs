use harness_application::{
    Application,
    ports::{SessionContext, SessionMutation, SessionRepository},
};
use harness_domain::{
    Failure,
    contracts::{CheckpointArgs, ReadArgs, Request, SearchArgs, TasksAction, TasksArgs},
    execution::OperationContext,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

struct Store {
    gate: Arc<Mutex<()>>,
}
impl SessionRepository for Store {
    fn checkpoint(&mut self, _: &CheckpointArgs) -> Result<SessionMutation, Failure> {
        Ok(SessionMutation {
            data: json!({"observedByHarness":{"checks":[]}}),
            snapshot: None,
        })
    }
    fn workflow_gate(&mut self, _: &str) -> Result<Arc<Mutex<()>>, Failure> {
        Ok(self.gate.clone())
    }
    fn acquire(&mut self, _: &str) -> Result<(), Failure> {
        Ok(())
    }
    fn release(&mut self, _: &str) {}
    fn open(&mut self, _: Option<&str>) -> Result<SessionContext, Failure> {
        unreachable!()
    }
    fn update_hashes(&mut self, _: &str, _: String, _: String) -> Result<(), Failure> {
        unreachable!()
    }
    fn tasks(&mut self, args: &TasksArgs) -> Result<SessionMutation, Failure> {
        Ok(SessionMutation {
            data: json!({"readable":true}),
            snapshot: matches!(args.action, Some(TasksAction::Update)).then(|| json!({})),
        })
    }
}
struct Disk {
    started: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}
impl harness_application::ports::workspace::WorkspaceMetadata for Disk {
    fn info(&self) -> Value {
        json!({})
    }
    fn rules(&self, _: &OperationContext) -> Result<String, Failure> {
        Ok(String::new())
    }
    fn capabilities(&self) -> Value {
        json!({})
    }
}
impl harness_application::ports::workspace::SourceQueries for Disk {
    fn read(&self, _: &ReadArgs, _: &OperationContext) -> Result<Value, Failure> {
        unreachable!()
    }
    fn search(&self, _: &SearchArgs, _: &OperationContext) -> Result<Value, Failure> {
        unreachable!()
    }
}
impl harness_application::ports::workspace::SourceEditing for Disk {}
impl harness_application::ports::workspace::WorkspaceDiagnostics for Disk {
    fn observe_workspace(&self, _: &OperationContext) -> Result<Value, Failure> {
        self.started.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(json!({"status":"captured","hash":"fixture"}))
    }
}
impl harness_application::ports::workspace::VersionControl for Disk {}
impl harness_application::ports::workspace::SqlAccess for Disk {}
impl harness_application::ports::workspace::ArtifactQueries for Disk {}
impl harness_application::ports::browser::BrowserInspection for Disk {}
impl harness_application::ports::workspace::SessionPersistence for Disk {
    fn persist_session(&self, _: &str, _: &Value, _: &OperationContext) -> Result<(), Failure> {
        self.started.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(())
    }
}
impl harness_application::ports::workspace::LogPersistence for Disk {}
impl harness_application::ports::workspace::Verification for Disk {}
fn request(update: bool) -> Request {
    Request::Tasks(TasksArgs {
        session: "session".into(),
        action: Some(if update {
            TasksAction::Update
        } else {
            TasksAction::Status
        }),
        phase: Some("analysis".into()),
        status: Some("completed".into()),
        note: None,
        after: None,
        limit: None,
    })
}

#[test]
fn slow_persistence_keeps_global_repository_readable_but_serializes_session_writers() {
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let app = Application::new(
        Disk {
            started: started_tx,
            release: Mutex::new(release_rx),
        },
        Store {
            gate: Arc::default(),
        },
    );
    let writer = app.clone();
    let worker = std::thread::spawn(move || writer.execute(request(true)));
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (send, receive) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let readable = app.execute(request(false));
        let busy = app.execute(request(true));
        send.send((readable, busy)).unwrap();
    });
    let before_release = receive.recv_timeout(Duration::from_secs(2));
    release_tx.send(()).unwrap();
    assert_eq!(worker.join().unwrap()["data"]["persisted"], true);
    reader.join().unwrap();
    let (readable, busy) =
        before_release.expect("Persistence must not retain the repository mutex");
    assert_eq!(readable["state"], "ok");
    assert_eq!(busy["diagnostics"][0]["code"], "SESSION_BUSY");
}

#[test]
fn workspace_refresh_keeps_global_repository_readable_and_holds_session_gate() {
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let app = Application::new(
        Disk {
            started: started_tx,
            release: Mutex::new(release_rx),
        },
        Store {
            gate: Arc::default(),
        },
    );
    let observer = app.clone();
    let worker = std::thread::spawn(move || {
        observer.execute(Request::Checkpoint(CheckpointArgs {
            session: "session".into(),
            goal: None,
            decisions: None,
            pending: None,
            refresh_freshness: Some(true),
        }))
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (send, receive) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let readable = app.execute(request(false));
        let busy = app.execute(request(true));
        send.send((readable, busy)).unwrap();
    });
    let before_release = receive.recv_timeout(Duration::from_secs(2));
    release_tx.send(()).unwrap();
    assert_eq!(worker.join().unwrap()["state"], "ok");
    reader.join().unwrap();
    let (readable, busy) =
        before_release.expect("Observation must not retain the repository mutex");
    assert_eq!(readable["state"], "ok");
    assert_eq!(busy["diagnostics"][0]["code"], "SESSION_BUSY");
}
