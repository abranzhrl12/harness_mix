use harness_application::{
    Application,
    ports::{SessionContext, SessionRepository},
};
use harness_domain::{
    Failure,
    contracts::{BootstrapArgs, DataQueryArgs, DetailArgs, Kind, ReadArgs, Request, SearchArgs},
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
impl harness_application::ports::workspace::WorkspaceMetadata for FakeWorkspace {
    fn info(&self) -> Value {
        json!({"root":"memory","languages":[],"manifestScope":"raíz del proyecto","indexed":false})
    }
    fn rules(&self, _: &OperationContext) -> Result<String, Failure> {
        Ok("reglas".into())
    }
    fn capabilities(&self) -> Value {
        json!({"node":false})
    }
}
impl harness_application::ports::workspace::SourceQueries for FakeWorkspace {
    fn read(&self, args: &ReadArgs, _: &OperationContext) -> Result<Value, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"path":args.path,"text":"memory"}))
    }
    fn search(&self, _: &SearchArgs, _: &OperationContext) -> Result<Value, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"matches":[]}))
    }
}
impl harness_application::ports::workspace::SourceEditing for FakeWorkspace {
    fn patch_preview(
        &self,
        _: &harness_domain::contracts::ReplaceArgs,
        _: &OperationContext,
    ) -> Result<Value, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"preview":true}))
    }
    fn replace(
        &self,
        _: &harness_domain::contracts::ReplaceArgs,
        _: &OperationContext,
    ) -> Result<Value, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"changed":true}))
    }
}
impl harness_application::ports::workspace::WorkspaceDiagnostics for FakeWorkspace {}
impl harness_application::ports::workspace::VersionControl for FakeWorkspace {}
impl harness_application::ports::workspace::SqlAccess for FakeWorkspace {}
impl harness_application::ports::workspace::ArtifactQueries for FakeWorkspace {}
impl harness_application::ports::workspace::SessionPersistence for FakeWorkspace {}
impl harness_application::ports::workspace::LogPersistence for FakeWorkspace {}
impl harness_application::ports::workspace::Verification for FakeWorkspace {
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
    recorded_writes: Arc<AtomicUsize>,
    exists: bool,
    rules: Option<String>,
    capabilities: Option<String>,
}
impl SessionRepository for MemorySessions {
    fn record(&mut self, _: &str, _: &str, _: &Value, _: &Value) -> Result<Option<Value>, Failure> {
        self.recorded_writes.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
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
            engine: None,
            warnings: None,
            session: Some(id.into()),
        })
    };
    assert_eq!(
        app.execute(request("missing"))["diagnostics"][0]["code"],
        "SESSION"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    app.execute(Request::Bootstrap(BootstrapArgs {
        compact: false,
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
        context_lines: None,
        external_scope: None,
        min_lines: None,
        globs: None,
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
        compact: false,
        session: None,
        include_rules: None,
    }));
    assert_eq!(first["data"]["rules"], "reglas");
    assert_eq!(first["data"]["tasks"]["total"], 4);
    let second = app.clone().execute(Request::Bootstrap(BootstrapArgs {
        compact: false,
        session: Some("memory-session".into()),
        include_rules: None,
    }));
    assert_eq!(second["data"]["rulesUnchanged"], true);
    assert_eq!(second["data"]["capabilitiesUnchanged"], true);
    assert!(second["data"].get("rules").is_none());
    let forced = app.execute(Request::Bootstrap(BootstrapArgs {
        compact: false,
        session: Some("memory-session".into()),
        include_rules: Some(true),
    }));
    assert_eq!(forced["data"]["rules"], "reglas");
}

#[test]
fn compact_bootstrap_reuses_hashes_and_keeps_catalog_retrievable() {
    let app = Application::new(
        FakeWorkspace {
            calls: Arc::default(),
        },
        MemorySessions::default(),
    );
    let compact = app.execute(Request::Bootstrap(BootstrapArgs {
        compact: true,
        session: None,
        include_rules: None,
    }));
    assert_eq!(compact["data"]["catalogCompact"], true);
    assert_eq!(compact["data"]["rules"], "reglas");
    let session = compact["data"]["session"].as_str().unwrap().to_owned();
    let rules_hash = compact["data"]["rulesHash"].clone();
    let capabilities_hash = compact["data"]["capabilitiesHash"].clone();
    assert_eq!(rules_hash, harness_domain::hash(b"reglas"));
    assert!(compact["data"].get("supportedTools").is_none());
    let detail_ref = compact["detail"]["ref"].as_str().unwrap().to_owned();
    let mut catalog_text = String::new();
    let mut offset = 0;
    loop {
        let page = app.execute(Request::Detail(DetailArgs {
            reference: detail_ref.clone(),
            offset: Some(offset),
            max_chars: Some(16_000),
            session: Some(session.clone()),
        }));
        catalog_text.push_str(page["data"]["text"].as_str().unwrap());
        match page["data"]["nextOffset"].as_u64() {
            Some(next_offset) => offset = usize::try_from(next_offset).unwrap(),
            None => break,
        }
    }
    let catalog: Value = serde_json::from_str(&catalog_text).unwrap();
    assert!(catalog["catalog"]["supportedTools"].is_array());
    assert!(catalog["catalog"]["routing"].is_array());
    assert_eq!(
        catalog["catalog"]["toolContract"]["contractHash"],
        compact["data"]["toolContract"]["contractHash"]
    );

    let reused = app.execute(Request::Bootstrap(BootstrapArgs {
        compact: true,
        session: Some(session.clone()),
        include_rules: None,
    }));
    assert_eq!(reused["data"]["catalogUnchanged"], true);
    assert_eq!(reused["data"]["rulesUnchanged"], true);
    assert_eq!(reused["data"]["capabilitiesUnchanged"], true);
    assert_eq!(reused["data"]["rulesHash"], rules_hash);
    assert_eq!(reused["data"]["capabilitiesHash"], capabilities_hash);
    assert!(reused["data"].get("rules").is_none());
    let forced = app.execute(Request::Bootstrap(BootstrapArgs {
        compact: true,
        session: Some(session),
        include_rules: Some(true),
    }));
    assert_eq!(forced["data"]["catalogCompact"], true);
    assert_eq!(forced["data"]["rules"], "reglas");
    assert_eq!(forced["data"]["rulesHash"], rules_hash);
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
        context_lines: None,
        external_scope: None,
        min_lines: None,
        globs: None,
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
        compact: false,
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

#[test]
fn data_query_rejects_mixed_result_ref_inputs_before_workspace_access() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Application::new(
        FakeWorkspace {
            calls: calls.clone(),
        },
        MemorySessions::default(),
    );
    let result = app.execute(Request::DataQuery(DataQueryArgs {
        path: Some("data.json".into()),
        data: None,
        result_ref: Some("saved-result".into()),
        query: "items".into(),
        max_chars: None,
        session: None,
    }));
    assert_eq!(result["state"], "error");
    assert_eq!(result["diagnostics"][0]["code"], "ARGUMENT");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn replace_preview_does_not_record_a_mutation_but_published_replace_does() {
    let calls = Arc::new(AtomicUsize::new(0));
    let sessions = MemorySessions::default();
    let recorded_writes = sessions.recorded_writes.clone();
    let app = Application::new(
        FakeWorkspace {
            calls: calls.clone(),
        },
        sessions,
    );
    let bootstrap = app.execute(Request::Bootstrap(BootstrapArgs {
        compact: false,
        session: None,
        include_rules: None,
    }));
    let session = bootstrap["data"]["session"].as_str().unwrap().to_owned();
    let args = |preview| harness_domain::contracts::ReplaceArgs {
        preview,
        session: Some(session.clone()),
        ..Default::default()
    };

    let preview = app.execute(Request::Replace(args(true)));
    assert_eq!(preview["data"]["preview"], true);
    assert_eq!(recorded_writes.load(Ordering::SeqCst), 0);

    let published = app.execute(Request::Replace(args(false)));
    assert_eq!(published["data"]["changed"], true);
    assert_eq!(recorded_writes.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
