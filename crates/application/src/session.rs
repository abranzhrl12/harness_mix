//! Session leases, persistence and bootstrap context. No native I/O dependencies.
use crate::Application;
use crate::SessionLease;
mod bootstrap;
use crate::ports;
use crate::ports::{SessionRepository, Workspace};
use harness_domain::{Failure, contracts::BootstrapArgs, execution::OperationContext, hash};
use serde_json::{Value, json};

impl<W: Workspace, S: SessionRepository> Application<W, S> {
    pub(super) fn with_session_write<T>(
        &self,
        id: Option<&str>,
        lease: &mut Option<SessionLease<S>>,
        context: &OperationContext,
        run: impl FnOnce(Option<&str>) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        self.lease(id, lease)?;
        let real_id = lease.as_ref().map(|l| l.id.as_str());
        let gate = real_id
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
        run(real_id)
    }
    pub(super) fn persist_update(&self, id: &str, mut update: ports::SessionMutation) -> Value {
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
    pub(super) fn record_result(
        &self,
        id: Option<&str>,
        tool: &str,
        args: &Value,
        result: &mut Value,
    ) {
        let Some(id) = id else {
            return;
        };
        let mut sessions = match self.sessions.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        let real_id = match sessions.resolve_id(id) {
            Ok(id) => id,
            Err(_) => return,
        };
        let snapshot = sessions.record(&real_id, tool, args, result);
        drop(sessions);
        let warning = match snapshot {
            Ok(Some(snapshot)) => self
                .workspace
                .persist_session(&real_id, &snapshot, &OperationContext::default())
                .err(),
            Ok(None) => None,
            Err(failure) => Some(failure),
        };
        if let Some((code, message)) = warning {
            result["sessionWarning"] =
                json!({"code":code,"message":message,"toolOutcomeUnchanged":true});
        }
    }
    pub(super) fn lease(
        &self,
        id: Option<&str>,
        lease: &mut Option<SessionLease<S>>,
    ) -> Result<(), Failure> {
        if let Some(id) = id {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| ("INTERNAL", "sesiones no disponibles"))?;
            let real_id = sessions.resolve_id(id)?;
            sessions.acquire(&real_id)?;
            *lease = Some(SessionLease {
                sessions: self.sessions.clone(),
                id: real_id,
            });
        }
        Ok(())
    }
    pub(super) fn bootstrap(
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
        let contract = self.tool_profile.metadata();
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
        let result = self.workspace.info();
        Ok(bootstrap::build_response(
            result,
            bootstrap::ResponseInput {
                session,
                args,
                rules,
                unchanged,
                same_capabilities,
                compact_reuse,
                available,
                contract,
                tool_profile: self.tool_profile,
            },
        ))
    }
}
