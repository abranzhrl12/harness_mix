use harness_application::{
    Application,
    ports::{SessionContext, SessionRepository, Workspace},
};
use harness_domain::{
    Failure,
    contracts::{BootstrapArgs, Kind, ReadArgs, Request, SearchArgs},
    execution::OperationContext,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

// These tests deliberately have no disk, process, Tokio or MCP dependency.
struct FakeWorkspace {
    calls: Arc<AtomicUsize>,
}
impl Workspace for FakeWorkspace {
    fn info(&self) -> Value {
        json!({"root":"memory","languages":[],"manifestScope":"raíz del proyecto","indexed":false})
    }
    fn rules(&self, _: &OperationContext) -> Result<String, Failure> {
        Ok("reglas".into())
    }
    fn capabilities(&self) -> Value {
        json!({"node":false})
    }
    fn read(&self, args: &ReadArgs, _: &OperationContext) -> Result<Value, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"path":args.path,"text":"memory"}))
    }
    fn search(&self, _: &SearchArgs, _: &OperationContext) -> Result<Value, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"matches":[]}))
    }
    fn verify(
        &self,
        _: &harness_domain::contracts::VerifyArgs,
        _: &OperationContext,
    ) -> Result<Value, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"passed":true}))
    }
}

#[derive(Default)]
struct MemorySessions {
    gate: Arc<std::sync::Mutex<()>>,
    active: Arc<AtomicUsize>,
    exists: bool,
    rules: Option<String>,
    capabilities: Option<String>,
}
impl SessionRepository for MemorySessions {
    fn tasks(
        &mut self,
        args: &harness_domain::contracts::TasksArgs,
    ) -> Result<harness_application::ports::SessionMutation, Failure> {
        Ok(harness_application::ports::SessionMutation {
            data: json!({"statusProvenance":"agent-declared"}),
            snapshot: matches!(
                args.action,
                Some(harness_domain::contracts::TasksAction::Update)
            )
            .then(|| json!({"snapshot":"memory"})),
        })
    }
    fn workflow_gate(&mut self, _: &str) -> Result<Arc<std::sync::Mutex<()>>, Failure> {
        Ok(self.gate.clone())
    }
    fn acquire(&mut self, id: &str) -> Result<(), Failure> {
        self.open(Some(id))?;
        self.active.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn release(&mut self, _: &str) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
    fn open(&mut self, id: Option<&str>) -> Result<SessionContext, Failure> {
        if id.is_some_and(|id| !self.exists || id != "memory-session") {
            return Err(("SESSION", "sesión inexistente"));
        }
        self.exists = true;
        Ok(SessionContext {
            id: "memory-session".into(),
            restored: false,
            rules_hash: self.rules.clone(),
            capabilities_hash: self.capabilities.clone(),
            task_summary: json!({"total":4}),
            persistence_warning: None,
        })
    }
    fn update_hashes(
        &mut self,
        _: &str,
        rules: String,
        capabilities: String,
    ) -> Result<(), Failure> {
        self.rules = Some(rules);
        self.capabilities = Some(capabilities);
        Ok(())
    }
}

#[test]
fn verification_admission_prevents_io_and_releases_session_lease() {
    let calls = Arc::new(AtomicUsize::new(0));
    let sessions = MemorySessions::default();
    let active = sessions.active.clone();
    let app = Application::new(
        FakeWorkspace {
            calls: calls.clone(),
        },
        sessions,
    );
    let request = |id: &str| {
        Request::Verify(harness_domain::contracts::VerifyArgs {
            language: "rust".into(),
            package: None,
            scope: Some("crate".into()),
            action: None,
            session: Some(id.into()),
        })
    };
    assert_eq!(
        app.execute(request("missing"))["diagnostics"][0]["code"],
        "SESSION"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    app.execute(Request::Bootstrap(BootstrapArgs {
        session: None,
        include_rules: None,
    }));
    assert_eq!(
        app.execute(request("memory-session"))["data"]["passed"],
        true
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    let context = OperationContext::default();
    context.cancel();
    assert_eq!(
        app.execute_with_context(request("memory-session"), context)["state"],
        "cancelled"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[test]
fn session_admission_runs_before_any_io_for_reads_and_searches() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Application::new(
        FakeWorkspace {
            calls: calls.clone(),
        },
        MemorySessions::default(),
    );
    let read = app.execute(Request::Read(ReadArgs {
        external_scope: None,
        path: "a.txt".into(),
        start: 1,
        end: 1,
        session: Some("missing".into()),
    }));
    assert_eq!(read["diagnostics"][0]["code"], "SESSION");
    assert_eq!(read["scope"], json!({"tool":"harness_read","path":"a.txt"}));
    let search = app.execute(Request::Search(SearchArgs {
        external_scope: None,
        min_lines: None,
        kind: Kind::Text,
        query: "a".into(),
        scope: None,
        limit: None,
        session: Some("missing".into()),
    }));
    assert_eq!(search["diagnostics"][0]["code"], "SESSION");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let read = app.execute(Request::Read(ReadArgs {
        external_scope: None,
        path: "a.txt".into(),
        start: 1,
        end: 1,
        session: None,
    }));
    assert_eq!(read["state"], "ok");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cloned_application_shares_sessions_and_rule_capability_cache() {
    let app = Application::new(
        FakeWorkspace {
            calls: Arc::default(),
        },
        MemorySessions::default(),
    );
    let first = app.execute(Request::Bootstrap(BootstrapArgs {
        session: None,
        include_rules: None,
    }));
    assert_eq!(first["data"]["rules"], "reglas");
    assert_eq!(first["data"]["tasks"]["total"], 4);
    let second = app.clone().execute(Request::Bootstrap(BootstrapArgs {
        session: Some("memory-session".into()),
        include_rules: None,
    }));
    assert_eq!(second["data"]["rulesUnchanged"], true);
    assert_eq!(second["data"]["capabilitiesUnchanged"], true);
    assert!(second["data"].get("rules").is_none());
    let forced = app.execute(Request::Bootstrap(BootstrapArgs {
        session: Some("memory-session".into()),
        include_rules: Some(true),
    }));
    assert_eq!(forced["data"]["rules"], "reglas");
}

fn read_request() -> Request {
    Request::Read(ReadArgs {
        external_scope: None,
        path: "a.txt".into(),
        start: 1,
        end: 1,
        session: None,
    })
}
fn search_request() -> Request {
    Request::Search(SearchArgs {
        external_scope: None,
        min_lines: None,
        kind: Kind::Text,
        query: "a".into(),
        scope: None,
        limit: None,
        session: None,
    })
}

#[test]
fn admission_bounds_pending_work_and_recovers_when_jobs_are_dropped() {
    let app = Application::new(
        FakeWorkspace {
            calls: Arc::default(),
        },
        MemorySessions::default(),
    );
    let jobs: Vec<_> = (0..4)
        .map(|_| {
            app.prepare(read_request(), OperationContext::default())
                .unwrap_or_else(|_| panic!("admission failed"))
        })
        .collect();
    assert_eq!(
        app.execute(read_request())["diagnostics"][0]["code"],
        "QUEUE_FULL"
    );
    drop(jobs);
    let searches: Vec<_> = (0..2)
        .map(|_| {
            app.prepare(search_request(), OperationContext::default())
                .unwrap_or_else(|_| panic!("admission failed"))
        })
        .collect();
    assert_eq!(
        app.execute(search_request())["diagnostics"][0]["code"],
        "QUEUE_FULL"
    );
    assert_eq!(app.execute(read_request())["state"], "ok");
    drop(searches);
    assert_eq!(app.execute(search_request())["state"], "ok");
}

#[test]
fn simultaneous_admission_never_exceeds_capacity_and_recovers() {
    let app = Application::new(
        FakeWorkspace {
            calls: Arc::default(),
        },
        MemorySessions::default(),
    );
    let barrier = Arc::new(std::sync::Barrier::new(9));
    let accepted = Arc::new(AtomicUsize::new(0));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let app = app.clone();
            let barrier = barrier.clone();
            let accepted = accepted.clone();
            std::thread::spawn(move || {
                let job = app.prepare(read_request(), OperationContext::default());
                if job.is_ok() {
                    accepted.fetch_add(1, Ordering::SeqCst);
                }
                barrier.wait();
                barrier.wait();
                drop(job);
            })
        })
        .collect();
    barrier.wait();
    let count = accepted.load(Ordering::SeqCst);
    barrier.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(count, 4);
    assert_eq!(app.execute(read_request())["state"], "ok");
}

#[test]
fn cancellation_and_timeout_are_distinct_and_do_not_leak_session_leases() {
    let sessions = MemorySessions::default();
    let active = sessions.active.clone();
    let app = Application::new(
        FakeWorkspace {
            calls: Arc::default(),
        },
        sessions,
    );
    app.execute(Request::Bootstrap(BootstrapArgs {
        session: None,
        include_rules: None,
    }));
    assert_eq!(active.load(Ordering::SeqCst), 0);
    let cancelled = OperationContext::default();
    cancelled.cancel();
    assert_eq!(
        app.execute_with_context(read_request(), cancelled)["state"],
        "cancelled"
    );
    let expired = OperationContext::with_timeout(std::time::Duration::ZERO);
    assert_eq!(
        app.execute_with_context(read_request(), expired)["state"],
        "timeout"
    );
    let read = Request::Read(ReadArgs {
        external_scope: None,
        path: "a.txt".into(),
        start: 1,
        end: 1,
        session: Some("memory-session".into()),
    });
    assert_eq!(app.execute(read)["state"], "ok");
    assert_eq!(active.load(Ordering::SeqCst), 0);
}
