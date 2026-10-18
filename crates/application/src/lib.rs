//! Use-case orchestration. Depends on domain and ports, never on MCP or OS adapters.
mod admission;
mod batch;
mod logging;
mod metrics;
pub mod ports;

use harness_domain::{
    Failure,
    contracts::{BootstrapArgs, Request},
    execution::OperationContext,
    hash,
    results::{Results, prefix, utf16_len},
};
use ports::{SessionRepository, Workspace};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub struct Application<W, S> {
    workspace: Arc<W>,
    sessions: Arc<Mutex<S>>,
    results: Arc<Mutex<Results>>,
    admission: Arc<admission::Admission>,
    metrics: Arc<metrics::Metrics>,
}

/// Owns admission before it is submitted to any worker pool; drop always frees it.
pub struct Operation<W, S> {
    application: Application<W, S>,
    request: Request,
    context: OperationContext,
    measurement: metrics::Measurement,
    _permit: admission::Permit,
}
impl<W: Workspace, S: SessionRepository> Operation<W, S> {
    pub fn request_id(&self) -> &str {
        self.measurement.request_id()
    }
    pub fn run(mut self) -> Value {
        let mut output = self
            .application
            .execute_admitted(self.request, &self.context);
        output["requestId"] = json!(self.measurement.request_id());
        self.measurement.finish_envelope(&output);
        if let Some(archive) = self.application.metrics.logs.archive() {
            let sequence = archive["lastSequence"].as_u64().unwrap_or(0);
            let saved = self.application.workspace.persist_log_archive(&archive);
            let saved_through = saved.as_ref().ok().copied();
            self.application.metrics.logs.saved(
                saved_through.unwrap_or(sequence),
                saved.as_ref().err().map(|f| f.0),
            );
            output["logPersistence"] = json!({"saved":saved.is_ok(),"requestedSequence":sequence,"savedThrough":saved_through});
            if let Err((code, _)) = saved {
                output["loggingWarning"] = json!({"code":code,"toolOutcomeUnchanged":true});
            }
        }
        output
    }
}

struct SessionLease<S: SessionRepository> {
    sessions: Arc<Mutex<S>>,
    id: String,
}
impl<S: SessionRepository> Drop for SessionLease<S> {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.release(&self.id);
        }
    }
}
impl<W, S> Clone for Application<W, S> {
    fn clone(&self) -> Self {
        Self {
            workspace: self.workspace.clone(),
            sessions: self.sessions.clone(),
            results: self.results.clone(),
            admission: self.admission.clone(),
            metrics: self.metrics.clone(),
        }
    }
}
impl<W: Workspace, S: SessionRepository> Application<W, S> {
    pub fn new(workspace: W, sessions: S) -> Self {
        let enabled = workspace.log_persistence_enabled();
        let history = if enabled {
            workspace.load_log_archive()
        } else {
            Ok(None)
        };
        let logs = logging::Log::configured(enabled, history);
        Self {
            workspace: Arc::new(workspace),
            sessions: Arc::new(Mutex::new(sessions)),
            results: Arc::default(),
            admission: Arc::default(),
            metrics: Arc::new(metrics::Metrics::with_logs(logs)),
        }
    }

    /// A single boundary for every transport: session admission, execution and envelope.
    pub fn execute(&self, request: Request) -> Value {
        self.execute_with_context(request, OperationContext::default())
    }

    pub fn execute_with_context(&self, request: Request, context: OperationContext) -> Value {
        match self.prepare(request, context) {
            Ok(operation) => operation.run(),
            Err(envelope) => envelope,
        }
    }

    pub fn prepare(
        &self,
        request: Request,
        context: OperationContext,
    ) -> Result<Operation<W, S>, Value> {
        let request_id = harness_domain::execution::new_request_id();
        let permit = context
            .check()
            .and_then(|()| {
                if matches!(&request, Request::Batch(_)) {
                    self.admission.acquire_batch()
                } else {
                    self.admission
                        .acquire(matches!(&request, Request::Search(_)))
                }
            })
            .map_err(|failure| {
                self.metrics
                    .rejected_with_id(request.name(), failure.0, &request_id);
                let mut output = self.failure(request.name(), &request.arguments(), failure);
                output["requestId"] = json!(request_id);
                output
            })?;
        let measurement = self.metrics.admitted_with_id(request.name(), &request_id);
        Ok(Operation {
            application: self.clone(),
            request,
            context,
            _permit: permit,
            measurement,
        })
    }

    pub fn failure(&self, name: &str, args: &Value, failure: Failure) -> Value {
        // Rejections/cancellation must not block the async transport on a worker's store lock.
        Results::default().envelope(name, args, Err(failure))
    }

    fn execute_admitted(&self, request: Request, context: &OperationContext) -> Value {
        let mutating = request.is_mutating();
        let name = request.name();
        let args = request.arguments();
        let mut lease = None;
        let result = context.check().and_then(|()| match request {
            Request::Explore(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.explore(&args, context)),
            Request::Bootstrap(args) => self.bootstrap(args, context, &mut lease),
            Request::ExternalScope(args) => {
                if matches!(
                    args.action,
                    harness_domain::contracts::ExternalScopeAction::List
                ) {
                    self.lease(Some(&args.session), &mut lease)
                        .and_then(|()| self.workspace.external_scope(&args, context))
                } else {
                    self.with_session_write(Some(&args.session), &mut lease, context, || {
                        self.workspace.external_scope(&args, context)
                    })
                }
            }
            Request::Config(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.config(&args, context)),
            Request::Logs(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.metrics.logs.read(&args)),
            Request::Batch(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.batch(args, context)),
            Request::Doctor(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.doctor(&args, context)),
            Request::GitStatus(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.git_status(&args, context)),
            Request::GitDiff(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.git_diff(&args, context)),
            Request::SqlStatus(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.sql_status(&args, context)),
            Request::SqlSearch(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.sql_search(&args, context)),
            Request::SqlDescribe(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.sql_describe(&args, context)),
            Request::SqlQuery(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.sql_query(&args, context)),
            Request::XlsxInspect(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.xlsx_inspect(&args, context)),
            Request::XlsxRead(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.xlsx_read(&args, context)),
            Request::Svg(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.svg(&args, context)),
            Request::Handover(args) => {
                if args.target_file.is_some() {
                    self.with_session_write(Some(&args.session), &mut lease, context, || {
                        let snapshot = self
                            .sessions
                            .lock()
                            .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                            .snapshot(&args.session)?;
                        self.workspace.handover(&args, &snapshot, context)
                    })
                } else {
                    self.lease(Some(&args.session), &mut lease).and_then(|()| {
                        let snapshot = self
                            .sessions
                            .lock()
                            .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                            .snapshot(&args.session)?;
                        self.workspace.handover(&args, &snapshot, context)
                    })
                }
            }
            Request::ProcessStatus(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.process_status(&args, context)),
            Request::Diagnostics(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.diagnostics(&args, context)),
            Request::DataQuery(mut args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| {
                    if let Some(reference) = args.result_ref.take() {
                        if args.path.is_some() || args.data.is_some() {
                            return Err(("ARGUMENT", "Use exactly one of path, data or resultRef"));
                        }
                        args.data = Some(
                            self.results
                                .lock()
                                .map_err(|_| ("INTERNAL", "Results unavailable"))?
                                .value(&reference)?,
                        );
                    }
                    self.workspace.data_query(&args, context)
                }),
            Request::Outline(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.outline(&args, context)),
            Request::ReadSymbol(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.read_symbol(&args, context)),
            Request::Replace(args) => {
                self.with_session_write(args.session.as_deref(), &mut lease, context, || {
                    let mut result = self.workspace.replace(&args, context)?;
                    self.record_result(
                        args.session.as_deref(),
                        name,
                        &serde_json::to_value(&args).unwrap(),
                        &mut result,
                    );
                    Ok(result)
                })
            }
            Request::Read(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.read(&args, context)),
            Request::Search(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.search(&args, context)),
            Request::Verify(args) => {
                self.with_session_write(args.session.as_deref(), &mut lease, context, || {
                    let mut result = self.workspace.verify(&args, context)?;
                    self.record_result(
                        args.session.as_deref(),
                        name,
                        &serde_json::to_value(&args).unwrap(),
                        &mut result,
                    );
                    Ok(result)
                })
            }
            Request::Tasks(args) => {
                if matches!(
                    args.action,
                    Some(harness_domain::contracts::TasksAction::Update)
                ) {
                    self.with_session_write(Some(&args.session), &mut lease, context, || {
                        let update = self
                            .sessions
                            .lock()
                            .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                            .tasks(&args)?;
                        Ok(self.persist_update(&args.session, update))
                    })
                } else {
                    self.lease(Some(&args.session), &mut lease).and_then(|()| {
                        self.sessions
                            .lock()
                            .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                            .tasks(&args)
                            .map(|update| update.data)
                    })
                }
            }
            Request::Checkpoint(args) => {
                if args.has_updates() || args.refresh_freshness == Some(true) {
                    self.with_session_write(Some(&args.session), &mut lease, context, || {
                        // Observe before declaring changes, without the global session lock.
                        // The per-session gate prevents checks being appended during the scan.
                        let observation = if args.refresh_freshness == Some(true) {
                            Some(self.workspace.observe_workspace(context)?)
                        } else {
                            None
                        };
                        context.check()?;
                        let mut sessions = self
                            .sessions
                            .lock()
                            .map_err(|_| ("INTERNAL", "Sessions unavailable"))?;
                        let mut update = sessions.checkpoint(&args)?;
                        if let Some(observation) = &observation {
                            sessions.invalidate_checks(&args.session, observation)?;
                        }
                        drop(sessions);
                        if let Some(observation) = observation {
                            harness_domain::freshness::refresh_checkpoint(
                                &mut update.data,
                                &observation,
                            );
                        }
                        Ok(self.persist_update(&args.session, update))
                    })
                } else {
                    self.lease(Some(&args.session), &mut lease).and_then(|()| {
                        self.sessions
                            .lock()
                            .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                            .checkpoint(&args)
                            .map(|update| update.data)
                    })
                }
            }
            Request::Detail(args) => {
                self.lease(args.session.as_deref(), &mut lease)
                    .and_then(|()| {
                        self.results
                            .lock()
                            .map_err(|_| ("INTERNAL", "almacén no disponible"))?
                            .get(
                                &args.reference,
                                args.offset.unwrap_or(0),
                                args.max_chars.unwrap_or(8000),
                            )
                    })
            }
        });
        let result = result.and_then(|value| {
            // Search can return an explicit partial result on cancellation/deadline.
            if !mutating
                && value.get("cancelled") != Some(&json!(true))
                && value.get("timedOut") != Some(&json!(true))
            {
                context.check()?;
            }
            Ok(value)
        });
        let result = result.map(|mut value| {
            if name == "harness_doctor" {
                value["metrics"] = self.metrics.snapshot();
                value["logging"] = self.metrics.logs.status();
                if !value["logging"]["persistenceError"].is_null() {
                    if let Some(checks)=value["checks"].as_array_mut() {checks.push(json!({"id":"logging:persistence","status":"warning","message":"Native log archive unavailable; memory logging remains active"}));}
                    value["counts"]["warning"]=json!(value["counts"]["warning"].as_u64().unwrap_or(0)+1);
                    if value["health"]=="ok" {value["health"]=json!("warning");}
                }
            } else if name == "harness_checkpoint" {
                let stats = self.metrics.snapshot();
                value["stats"] = stats.clone();
                if let Some(full) = value.get_mut("_detail") {
                    full["stats"] = stats;
                }
            }
            value
        });
        let output = self.output(name, &args, result);
        drop(lease);
        output
    }

    fn with_session_write<T>(
        &self,
        id: Option<&str>,
        lease: &mut Option<SessionLease<S>>,
        context: &OperationContext,
        run: impl FnOnce() -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        self.lease(id, lease)?;
        let gate = id
            .map(|id| {
                self.sessions
                    .lock()
                    .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                    .workflow_gate(id)
            })
            .transpose()?;
        let _guard = gate
            .as_ref()
            .map(|gate| {
                gate.try_lock().map_err(|_| {
                    (
                        "SESSION_BUSY",
                        "Another workflow write or verification is active in this session",
                    )
                })
            })
            .transpose()?;
        context.check()?;
        run()
    }

    fn persist_update(&self, id: &str, mut update: ports::SessionMutation) -> Value {
        if let Some(snapshot) = update.snapshot {
            match self
                .workspace
                .persist_session(id, &snapshot, &OperationContext::default())
            {
                Ok(()) => update.data["persisted"] = json!(true),
                Err((code, message)) => {
                    update.data["persisted"] = json!(false);
                    update.data["persistenceWarning"] =
                        json!({"code":code,"message":message,"memoryUpdated":true});
                }
            }
        }
        update.data
    }

    fn record_result(&self, id: Option<&str>, tool: &str, args: &Value, result: &mut Value) {
        let Some(id) = id else {
            return;
        };
        let snapshot = self
            .sessions
            .lock()
            .map_err(|_| ("INTERNAL", "Sessions unavailable"))
            .and_then(|mut sessions| sessions.record(id, tool, args, result));
        let warning = match snapshot {
            Ok(Some(snapshot)) => self
                .workspace
                .persist_session(id, &snapshot, &OperationContext::default())
                .err(),
            Ok(None) => None,
            Err(failure) => Some(failure),
        };
        if let Some((code, message)) = warning {
            result["sessionWarning"] =
                json!({"code":code,"message":message,"toolOutcomeUnchanged":true});
        }
    }

    fn output(&self, name: &str, args: &Value, result: Result<Value, Failure>) -> Value {
        match self.results.lock() {
            Ok(mut results) => results.envelope(name, args, result),
            Err(_) => Results::default().envelope(
                name,
                args,
                Err(("INTERNAL", "almacén de resultados no disponible")),
            ),
        }
    }

    fn lease(&self, id: Option<&str>, lease: &mut Option<SessionLease<S>>) -> Result<(), Failure> {
        if let Some(id) = id {
            self.sessions
                .lock()
                .map_err(|_| ("INTERNAL", "sesiones no disponibles"))?
                .acquire(id)?;
            *lease = Some(SessionLease {
                sessions: self.sessions.clone(),
                id: id.into(),
            });
        }
        Ok(())
    }

    fn bootstrap(
        &self,
        args: BootstrapArgs,
        context: &OperationContext,
        lease: &mut Option<SessionLease<S>>,
    ) -> Result<Value, Failure> {
        let is_new = args.session.is_none();
        let session = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| ("INTERNAL", "sesiones no disponibles"))?;
            let session = sessions.open(args.session.as_deref())?;
            sessions.acquire(&session.id)?;
            *lease = Some(SessionLease {
                sessions: self.sessions.clone(),
                id: session.id.clone(),
            });
            session
        };
        context.check()?;
        let rules = self.workspace.rules(context)?;
        context.check()?;
        let rules_hash = hash(rules.as_bytes());
        let unchanged = session.rules_hash.as_ref() == Some(&rules_hash)
            && !args.include_rules.unwrap_or(false);
        let available = self.workspace.capabilities();
        let contract = harness_domain::tool_contract::metadata("rust");
        let capabilities_hash = hash(
            json!({"executables":available,"contract":contract})
                .to_string()
                .as_bytes(),
        );
        let same_capabilities = session.capabilities_hash.as_ref() == Some(&capabilities_hash);
        let compact_reuse = !is_new && !session.restored && same_capabilities && unchanged;
        context.check()?;
        self.sessions
            .lock()
            .map_err(|_| ("INTERNAL", "sesiones no disponibles"))?
            .update_hashes(&session.id, rules_hash.clone(), capabilities_hash.clone())?;
        let mut result = self.workspace.info();
        result["sessionId"] = json!(session.id);
        result["rulesHash"] = json!(rules_hash);
        result["capabilitiesHash"] = json!(capabilities_hash);
        result["sessionRestored"] = json!(session.restored);
        result["implementation"] = json!("rust-experimental");
        result["toolContract"] = contract;
        result["backend"] = json!({"name":"rust","version":result.get("implementationVersion").cloned().unwrap_or(Value::Null)});
        result["logging"] = self.metrics.logs.status();
        result["supportedTools"] = json!(harness_domain::tool_contract::tool_names("rust"));
        result["limitations"] = json!([
            "Edición y navegación dependen del lenguaje y backend; consultar las capacidades declaradas de cada herramienta",
            "Explore: índice existente con selectores exactos; syncIndex es opt-in y no certifica cobertura global",
            "Búsqueda text/files/file_size y syntax de declaraciones Rust (kind[:nombre]); sin patrones AST generales. Scopes Windows solo lectura",
            "Refresh solo de cobertura local; métricas por instancia, no históricas ni específicas de sesión",
            "Verificación Rust/Go/Python/JavaScript/.NET parcial; .NET exige .csproj/.sln, SDK y restore previo. Ejecuta código del proyecto, no es sandbox",
            "Git solo lectura; requiere git instalado y raíz Git o worktree vinculado registrado",
            "No sustituye el servidor actual"
        ]);
        result["executionPolicy"] = json!({"maxActive":admission::MAX_ACTIVE,"maxSearches":admission::MAX_SEARCHES,"maxBatchCoordinators":admission::MAX_BATCHES,"maxChildrenPerBatch":2,"queue":"reject_when_full","timeoutMs":30000,"cancellation":"cooperative"});
        if is_new {
            result["tasks"] = session.task_summary;
        }
        if let Some(warning) = session.persistence_warning {
            result["persistenceWarning"] = warning;
        }
        if is_new || args.include_rules.unwrap_or(false) || !same_capabilities {
            result["routing"] = json!(harness_domain::tool_contract::routing("rust"));
            result["workflow"] =
                harness_domain::tool_contract::manifest()["llmPolicy"]["workflows"].clone();
            result["usageRules"] =
                harness_domain::tool_contract::manifest()["llmPolicy"]["rules"].clone();
        }
        if same_capabilities {
            result["capabilitiesUnchanged"] = json!(true);
        } else {
            result["available"] = available;
        }
        if unchanged {
            result["rulesUnchanged"] = json!(true);
        } else {
            result["rules"] = json!(prefix(&rules, 12000));
            if utf16_len(&rules) > 12000 {
                result["truncated"] = json!(true);
                result["_detail"] = json!({"rules":rules});
            }
        }
        if compact_reuse {
            // The contract/rules hash was checked above; retain a retrievable
            // catalog without resending the same tool lists and policy text.
            let catalog = json!({"supportedTools":result["supportedTools"],"toolContract":result["toolContract"],"executionPolicy":result["executionPolicy"],"limitations":result["limitations"],"routing":harness_domain::tool_contract::routing("rust"),"workflow":harness_domain::tool_contract::manifest()["llmPolicy"]["workflows"],"usageRules":harness_domain::tool_contract::manifest()["llmPolicy"]["rules"]});
            let compact = json!({"backend":"rust","contractVersion":result["toolContract"]["contractVersion"],"contractHash":result["toolContract"]["contractHash"],"unchanged":true});
            result["toolContract"] = compact;
            result["catalogUnchanged"] = json!(true);
            for key in ["supportedTools", "executionPolicy", "limitations"] {
                result
                    .as_object_mut()
                    .expect("bootstrap object")
                    .remove(key);
            }
            result["_detail"] = json!({"catalog":catalog});
        }
        Ok(result)
    }
}
