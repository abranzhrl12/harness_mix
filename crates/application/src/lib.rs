//! Use-case orchestration. Depends on domain and ports, never on MCP or OS adapters.
mod admission;
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
}

/// Owns admission before it is submitted to any worker pool; drop always frees it.
pub struct Operation<W, S> {
    application: Application<W, S>,
    request: Request,
    context: OperationContext,
    _permit: admission::Permit,
}
impl<W: Workspace, S: SessionRepository> Operation<W, S> {
    pub fn run(self) -> Value {
        self.application
            .execute_admitted(self.request, &self.context)
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
        }
    }
}
impl<W: Workspace, S: SessionRepository> Application<W, S> {
    pub fn new(workspace: W, sessions: S) -> Self {
        Self {
            workspace: Arc::new(workspace),
            sessions: Arc::new(Mutex::new(sessions)),
            results: Arc::default(),
            admission: Arc::default(),
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
        let permit = context
            .check()
            .and_then(|()| {
                self.admission
                    .acquire(matches!(&request, Request::Search(_)))
            })
            .map_err(|failure| self.failure(request.name(), &request.arguments(), failure))?;
        Ok(Operation {
            application: self.clone(),
            request,
            context,
            _permit: permit,
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
            Request::Bootstrap(args) => self.bootstrap(args, context, &mut lease),
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
                if args.has_updates() {
                    self.with_session_write(Some(&args.session), &mut lease, context, || {
                        let update = self
                            .sessions
                            .lock()
                            .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                            .checkpoint(&args)?;
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
        let capabilities_hash = hash(available.to_string().as_bytes());
        let same_capabilities = session.capabilities_hash.as_ref() == Some(&capabilities_hash);
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
        result["supportedTools"] = json!([
            "harness_bootstrap",
            "harness_read",
            "harness_detail",
            "harness_search",
            "harness_verify",
            "harness_replace",
            "harness_outline",
            "harness_read_symbol",
            "harness_tasks",
            "harness_checkpoint"
        ]);
        result["limitations"] = json!([
            "Edición Windows solo Rust/JSON/texto; navegación de símbolos solo Rust, sin SQL ni otros parsers",
            "Navegación sintáctica: sin locales, expansión de macros ni relaciones CodeGraph",
            "Búsqueda solo text/files; sin syntax/file_size ni scopes externos",
            "Sin refresh de vigencia ni métricas; timeline solo en memoria del proceso",
            "Verificación solo Rust; ejecuta código del proyecto, no es un sandbox",
            "No sustituye el servidor actual"
        ]);
        result["executionPolicy"] = json!({"maxActive":admission::MAX_ACTIVE,"maxSearches":admission::MAX_SEARCHES,"queue":"reject_when_full","timeoutMs":30000,"cancellation":"cooperative"});
        if is_new {
            result["tasks"] = session.task_summary;
        }
        if let Some(warning) = session.persistence_warning {
            result["persistenceWarning"] = warning;
        }
        if is_new || args.include_rules.unwrap_or(false) {
            result["routing"] = json!([
                {"need":"plan declarado y timeline observado de sesión","tool":"harness_tasks"},
                {"need":"objetivo, decisiones y resumen histórico sin certificar vigencia","tool":"harness_checkpoint"},
                {"need":"firmas de declaraciones Rust por nombre","tool":"harness_outline"},
                {"need":"cuerpo exacto de símbolo Rust con hash","tool":"harness_read_symbol","chooseView":"full | signature"},
                {"need":"fragmento de hasta 100 líneas con hash SHA-256","tool":"harness_read"},
                {"need":"texto literal o nombres de archivos","tool":"harness_search","chooseKind":"text | files"},
                {"need":"recuperar detalles paginados sin repetir la operación","tool":"harness_detail"}]);
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
        Ok(result)
    }
}
