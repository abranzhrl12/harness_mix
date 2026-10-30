use super::*;
use harness_application::ports::SessionContext;
use harness_application::{Application, ports::SessionRepository};
use harness_domain::Failure;
use harness_domain::execution::OperationContext;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::oneshot;

#[test]
fn server_instances_keep_presentation_options_independent() {
    let application = || {
        Application::new(
            WaitingWorkspace {
                started: Mutex::new(None),
                finished: Mutex::new(None),
            },
            TrackingSessions(Arc::new(AtomicUsize::new(0)), Arc::default()),
        )
    };

    let eager_context = McpServer::new(application(), None)
        .with_eager_tools(true)
        .with_context_presentation(true);
    let lazy_plain = McpServer::new(application(), None)
        .with_eager_tools(false)
        .with_context_presentation(false);

    assert!(eager_context.eager_tools);
    assert!(eager_context.context_presentation);
    assert!(!lazy_plain.eager_tools);
    assert!(!lazy_plain.context_presentation);
}

#[test]
fn server_guides_relationship_queries_when_watcher_lock_is_busy() {
    let application = Application::new(
        WaitingWorkspace {
            started: Mutex::new(None),
            finished: Mutex::new(None),
        },
        TrackingSessions(Arc::new(AtomicUsize::new(0)), Arc::default()),
    );
    let server = McpServer::new(application, None);
    let info = rmcp::ServerHandler::get_info(&server);
    let instructions = info.instructions.unwrap();
    assert!(instructions.contains("use harness_explore first"));
    assert!(instructions.contains("SYNC_WATCH_BUSY"));
    assert!(instructions.contains("owner and its health are unknown"));
    assert!(instructions.contains("verify uncertain or missing relations against source"));
}

#[test]
fn generated_tool_router_registers_browser_with_object_discriminator_schema() {
    let application = Application::new(
        WaitingWorkspace {
            started: Mutex::new(None),
            finished: Mutex::new(None),
        },
        TrackingSessions(Arc::new(AtomicUsize::new(0)), Arc::default()),
    );
    let server = McpServer::new(application, None);
    let router = server.profile_router();
    let browser = router
        .get("harness_browser")
        .expect("browser tool registered");
    let schema = serde_json::to_value(&browser.input_schema).unwrap();

    assert_eq!(schema["type"], "object", "{schema}");
    assert!(schema["oneOf"].is_array(), "{schema}");
    assert!(
        schema["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .all(|variant| { variant["properties"]["action"]["type"] == "string" }),
        "{schema}"
    );
}

struct WaitingWorkspace {
    started: Mutex<Option<oneshot::Sender<()>>>,
    finished: Mutex<Option<oneshot::Sender<()>>>,
}
impl harness_application::ports::workspace::WorkspaceMetadata for WaitingWorkspace {
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
impl harness_application::ports::browser::BrowserInspection for WaitingWorkspace {}
impl harness_application::ports::workspace::SourceQueries for WaitingWorkspace {
    fn read(&self, _: &ReadArgs, context: &OperationContext) -> Result<Value, Failure> {
        self.started
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        while context.check().is_ok() {
            std::thread::park_timeout(Duration::from_millis(1));
        }
        self.finished
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        context.check()?;
        unreachable!()
    }
    fn search(&self, _: &SearchArgs, _: &OperationContext) -> Result<Value, Failure> {
        unreachable!()
    }
}
impl harness_application::ports::workspace::SourceEditing for WaitingWorkspace {
    fn replace(
        &self,
        _: &harness_domain::contracts::ReplaceArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.started
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        // Simulate cancellation after the publication has begun.
        while context.check().is_ok() {
            std::thread::park_timeout(Duration::from_millis(1));
        }
        self.finished
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        Ok(json!({"changed":true,"commitStatus":"committed"}))
    }
}
impl harness_application::ports::workspace::SourcePreviews for WaitingWorkspace {}
impl harness_application::ports::workspace::IgnoreManagement for WaitingWorkspace {}
impl harness_application::ports::workspace::WorkspaceDiagnostics for WaitingWorkspace {}
impl harness_application::ports::workspace::VersionControl for WaitingWorkspace {}
impl harness_application::ports::workspace::SqlAccess for WaitingWorkspace {}
impl harness_application::ports::workspace::ArtifactQueries for WaitingWorkspace {}
impl harness_application::ports::workspace::SessionPersistence for WaitingWorkspace {}
impl harness_application::ports::workspace::LogPersistence for WaitingWorkspace {}
impl harness_application::ports::workspace::Verification for WaitingWorkspace {}
struct TrackingSessions(Arc<AtomicUsize>, Arc<Mutex<()>>);
impl SessionRepository for TrackingSessions {
    fn workflow_gate(&mut self, _: &str) -> Result<Arc<Mutex<()>>, Failure> {
        Ok(self.1.clone())
    }
    fn acquire(&mut self, _: &str) -> Result<(), Failure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn release(&mut self, _: &str) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
    fn open(&mut self, _: Option<&str>) -> Result<SessionContext, Failure> {
        unreachable!()
    }
    fn update_hashes(&mut self, _: &str, _: String, _: String) -> Result<(), Failure> {
        unreachable!()
    }
}

#[tokio::test]
async fn mutation_cancellation_waits_for_definitive_commit_outcome() {
    let (started_tx, started_rx) = oneshot::channel();
    let (finished_tx, finished_rx) = oneshot::channel();
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let active = Arc::new(AtomicUsize::new(0));
    let application = Application::new(
        WaitingWorkspace {
            started: Mutex::new(Some(started_tx)),
            finished: Mutex::new(Some(finished_tx)),
        },
        TrackingSessions(active.clone(), Arc::default()),
    );
    let request = Request::Replace(harness_domain::contracts::ReplaceArgs {
        bom: None,
        path: "file.txt".into(),
        hash: "unused-by-fake".into(),
        line: None,
        chunks: None,
        old_text: "old".into(),
        new_text: "new".into(),
        allow_unvalidated_text: false,
        preview: false,
        session: Some("session".into()),
    });
    let task = tokio::spawn(async move {
        execute(
            &application,
            request,
            async {
                let _ = cancel_rx.await;
            },
            false,
            false,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .unwrap()
        .unwrap();
    cancel_tx.send(()).unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    finished_rx.await.unwrap();
    assert_eq!(response.structured_content.as_ref().unwrap()["state"], "ok");
    assert_eq!(
        response.structured_content.as_ref().unwrap()["data"]["changed"],
        true
    );
    // Batch metadata inspection must not add placeholders to a single result.
    let data = &response.structured_content.as_ref().unwrap()["data"];
    assert!(data.get("results").is_none());
    assert!(data.get("_detail").is_none());
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancellation_and_handler_drop_stop_workers_and_release_session_leases() {
    for abort_handler in [false, true] {
        let (started_tx, started_rx) = oneshot::channel();
        let (finished_tx, finished_rx) = oneshot::channel();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let active = Arc::new(AtomicUsize::new(0));
        let application = Application::new(
            WaitingWorkspace {
                started: Mutex::new(Some(started_tx)),
                finished: Mutex::new(Some(finished_tx)),
            },
            TrackingSessions(active.clone(), Arc::default()),
        );
        let recovered = application.clone();
        let request = Request::Read(ReadArgs {
            external_scope: None,
            path: "waiting".into(),
            start: 1,
            end: 1,
            compact: false,
            line_numbers: false,
            session: Some("session".into()),
        });
        let handler = tokio::spawn(async move {
            execute(
                &application,
                request,
                async {
                    let _ = cancel_rx.await;
                },
                false,
                false,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(3), started_rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(active.load(Ordering::SeqCst), 1);
        if abort_handler {
            handler.abort();
            let _ = handler.await;
        } else {
            cancel_tx.send(()).unwrap();
            let result = tokio::time::timeout(Duration::from_secs(3), handler)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(result.structured_content.unwrap()["state"], "cancelled");
        }
        tokio::time::timeout(Duration::from_secs(3), finished_rx)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while active.load(Ordering::SeqCst) != 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        // All four admissions are available again after physical worker completion.
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let jobs: Result<Vec<_>, _> = (0..4)
                    .map(|_| {
                        recovered.prepare(
                            Request::Detail(DetailArgs {
                                reference: "unused".into(),
                                offset: None,
                                max_chars: None,
                                session: None,
                            }),
                            OperationContext::default(),
                        )
                    })
                    .collect();
                if let Ok(jobs) = jobs {
                    drop(jobs);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("leaked admission");
    }
}
