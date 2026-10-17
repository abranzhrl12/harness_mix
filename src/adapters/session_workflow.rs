//! In-memory workflow operations; snapshots are written by Workspace outside the repository lock.
use super::sessions::{self, Session, Sessions};
use harness_application::ports::SessionMutation;
use harness_domain::{
    Failure,
    contracts::{CheckpointArgs, TasksAction, TasksArgs},
    results::{prefix, utf16_len},
};
use serde_json::{Value, json};

fn tail(items: &[Value], count: usize) -> &[Value] {
    &items[items.len().saturating_sub(count)..]
}

fn bounded(value: &Value) -> Result<&str, Failure> {
    value
        .as_str()
        .filter(|s| utf16_len(s) <= 1000)
        .ok_or(("SESSION_RECORD", "Observed field exceeds persistence limit"))
}

impl Session {
    fn snapshot(&self) -> Result<Value, Failure> {
        let mut state = self.state.clone();
        state.phases = self.tasks.phases();
        state.edits.extend(self.observed_edits.clone());
        state.edits = tail(&state.edits, 50).to_vec();
        state.checks.extend(self.observed_checks.clone());
        state.checks = tail(&state.checks, 20).to_vec();
        sessions::validate(&mut state)?;
        serde_json::to_value(state)
            .map_err(|_| ("PERSISTENCE", "Cannot serialize session snapshot"))
    }
    fn checkpoint_data(&self, id: &str) -> Value {
        let historical = self
            .restored
            .then(|| json!({"modifiedFiles":self.state.edits,"checks":self.historical_checks()}));
        let full = json!({"sessionId":id,"restored":self.restored,"historical":historical,
            "persistenceScope":"objetivo, decisiones, fases y hechos históricos; timeline solo del proceso actual",
            "declaredByAgent":self.state.declared,"tasks":self.summary(),
            "stats":{"available":false,"reason":"attached_by_application_boundary"},
            "observedByHarness":{"modifiedFiles":self.observed_edits,"checks":self.observed_checks,"omittedFiles":0,"omittedChecks":0},
            "coverage":"solo operaciones de esta sesión realizadas a través del arnés",
            "compaction":"el cliente controla el historial; este checkpoint no lo poda",
            "freshness":{"available":false,"reason":"full_input_coverage_not_tracked","checksAreHistorical":self.restored,"invalidationTracked":true}});
        let mut compact = full.clone();
        compact["declaredByAgent"] = json!({"goal":prefix(&self.state.declared.goal,500),
            "decisions":self.state.declared.decisions.iter().skip(self.state.declared.decisions.len().saturating_sub(3)).map(|s|prefix(s,200)).collect::<Vec<_>>(),
            "pending":self.state.declared.pending.iter().take(5).map(|s|prefix(s,150)).collect::<Vec<_>>()});
        compact["declaredOmissions"] = json!({"decisions":self.state.declared.decisions.len().saturating_sub(3),"pending":self.state.declared.pending.len().saturating_sub(5)});
        compact["observedByHarness"] = json!({"modifiedFiles":tail(&self.observed_edits,8),"checks":tail(&self.observed_checks,5),
            "omittedFiles":self.observed_edits.len().saturating_sub(8),"omittedChecks":self.observed_checks.len().saturating_sub(5)});
        if self.restored {
            compact["historical"] = json!({"modifiedFiles":tail(&self.state.edits,8),"checks":tail(&self.historical_checks(),5),
                "omittedFiles":self.state.edits.len().saturating_sub(8),"omittedChecks":self.state.checks.len().saturating_sub(5)});
        }
        compact["_detail"] = full;
        compact
    }

    fn historical_checks(&self) -> Vec<Value> {
        self.state
            .checks
            .iter()
            .cloned()
            .map(|mut check| {
                check["current"] = Value::Null;
                check["historical"] = json!(true);
                check
            })
            .collect()
    }
}

impl Sessions {
    pub(super) fn workflow_tasks(&mut self, args: &TasksArgs) -> Result<SessionMutation, Failure> {
        let session = self.get(&args.session)?;
        let mut snapshot = None;
        let data = match args.action {
            Some(TasksAction::Timeline) => session
                .tasks
                .timeline(args.after.unwrap_or(0), args.limit.unwrap_or(10))?,
            Some(TasksAction::Update) => {
                let phase = args
                    .phase
                    .as_deref()
                    .ok_or(("TASK", "update requires phase"))?;
                let status = args
                    .status
                    .as_deref()
                    .ok_or(("TASK", "update requires status"))?;
                if session.tasks.update(
                    phase,
                    status,
                    args.note.as_deref().unwrap_or(""),
                    &sessions::now(),
                )? {
                    session.state.phases = session.tasks.phases();
                    snapshot = Some(session.snapshot()?);
                }
                session.summary()
            }
            _ => session.summary(),
        };
        Ok(SessionMutation { data, snapshot })
    }

    pub(super) fn workflow_checkpoint(
        &mut self,
        args: &CheckpointArgs,
    ) -> Result<SessionMutation, Failure> {
        let valid = |list: &Option<Vec<String>>| {
            list.as_ref()
                .is_none_or(|items| items.len() <= 20 && items.iter().all(|s| utf16_len(s) <= 500))
        };
        if args.goal.as_ref().is_some_and(|s| utf16_len(s) > 1000)
            || !valid(&args.decisions)
            || !valid(&args.pending)
        {
            return Err((
                "CHECKPOINT",
                "Declared state exceeds limits (goal 1000, lists 20 x 500 UTF-16 units)",
            ));
        }
        let session = self.get(&args.session)?;
        let changed = args
            .goal
            .as_ref()
            .is_some_and(|goal| goal != &session.state.declared.goal)
            || args
                .decisions
                .as_ref()
                .is_some_and(|items| items != &session.state.declared.decisions)
            || args
                .pending
                .as_ref()
                .is_some_and(|items| items != &session.state.declared.pending);
        if let Some(goal) = &args.goal {
            session.state.declared.goal = goal.clone();
        }
        if let Some(items) = &args.decisions {
            session.state.declared.decisions = items.clone();
        }
        if let Some(items) = &args.pending {
            session.state.declared.pending = items.clone();
        }
        Ok(SessionMutation {
            data: session.checkpoint_data(&args.session),
            snapshot: changed.then(|| session.snapshot()).transpose()?,
        })
    }

    pub(super) fn session_snapshot(&mut self, id: &str) -> Result<Value, Failure> {
        let session = self.get(id)?;
        Ok(session.checkpoint_data(id)["_detail"].clone())
    }

    pub(super) fn record_observed(
        &mut self,
        id: &str,
        tool: &str,
        args: &Value,
        result: &Value,
    ) -> Result<Option<Value>, Failure> {
        let session = self.get(id)?;
        let time = sessions::now();
        if tool == "harness_replace" && result["changed"] == true {
            let path = bounded(&args["path"])?;
            let hash = bounded(&result["hash"])?;
            for check in &mut session.observed_checks {
                harness_domain::freshness::invalidate(check, "harness_edit_applied", &time);
            }
            session.observed_edits.retain(|item| item["path"] != path);
            session.observed_edits.push(json!({"path":path,"hash":hash,"time":time,"syntax":result["syntax"],"postcheckVerified":result["verified"]}));
            if session.observed_edits.len() > 50 {
                session.observed_edits.remove(0);
            }
            session.tasks.event(
                "edit_applied",
                &time,
                json!({"path":path,"source":"harness-observed"}),
            );
        } else if tool == "harness_verify" {
            let language = bounded(&args["language"])?;
            let scope = args["scope"].as_str().unwrap_or(".");
            if utf16_len(scope) > 1000 {
                return Err((
                    "SESSION_RECORD",
                    "Verification scope exceeds persistence limit",
                ));
            }
            let ok = result["passed"] == true;
            let observation = &result["workspaceObservation"];
            harness_domain::freshness::invalidate_observed_checks(
                &mut session.observed_checks,
                &json!({"status":"captured","hash":observation["after"],"coverage":observation["coverage"],"capturedAt":time}),
            );
            let mut check = json!({"language":language,"scope":scope,"action":result["action"],"ok":ok,"time":time,"current":null,"workspaceObservation":observation});
            if observation["status"] == "changed" {
                harness_domain::freshness::invalidate(
                    &mut check,
                    "workspace_changed_during_verification",
                    &time,
                );
            }
            session.observed_checks.push(check);
            if session.observed_checks.len() > 20 {
                session.observed_checks.remove(0);
            }
            session.tasks.event(
                "verification_finished",
                &time,
                json!({"language":language,"scope":scope,"ok":ok,"source":"harness-observed"}),
            );
        } else {
            return Ok(None);
        }
        session.snapshot().map(Some)
    }
}
