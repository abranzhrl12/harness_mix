//! Typed request dispatch; admission and result wrapping remain in the application.
use crate::Application;
use crate::ports::{SessionRepository, Workspace};
use harness_domain::{contracts::Request, execution::OperationContext};
use serde_json::{Value, json};

impl<W: Workspace, S: SessionRepository> Application<W, S> {
    pub(super) fn execute_admitted(&self, request: Request, context: &OperationContext) -> Value {
        let mutating = request.is_mutating();
        let name = request.name();
        let args = request.arguments();
        let mut lease = None;
        let result = context.check().and_then(|()| match request {
            Request::Explore(args) => {
                self.lease(args.session.as_deref(), &mut lease)
                    .and_then(|()| {
                        let impact =
                            args.view == Some(harness_domain::contracts::ExploreView::Impact);
                        self.workspace.explore(&args, context).map(|result| {
                            if impact {
                                harness_domain::assistance::impact(&args, result)
                            } else {
                                result
                            }
                        })
                    })
            }
            Request::Bootstrap(args) => self.bootstrap(args, context, &mut lease),
            Request::ExternalScope(args) => {
                if matches!(
                    args.action,
                    harness_domain::contracts::ExternalScopeAction::List
                ) {
                    self.lease(Some(&args.session), &mut lease)
                        .and_then(|()| self.workspace.external_scope(&args, context))
                } else {
                    self.with_session_write(Some(&args.session), &mut lease, context, |_| {
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
            Request::GitStatus(args) => {
                self.lease(args.session.as_deref(), &mut lease)
                    .and_then(|()| {
                        let mut result = self.workspace.git_status(&args, context)?;
                        let mut edits = Vec::new();
                        if let Some(id) = args.session.as_deref() {
                            let snapshot = self
                                .sessions
                                .lock()
                                .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                                .snapshot(id)?;
                            if let Some(previous) =
                                snapshot["historical"]["modifiedFiles"].as_array()
                            {
                                edits.extend(previous.iter().cloned());
                            }
                            if let Some(observed) =
                                snapshot["observedByHarness"]["modifiedFiles"].as_array()
                            {
                                edits.extend(observed.iter().cloned());
                            }
                        }
                        result["sessionChanges"] =
                            self.workspace.git_session_changes(&edits, context)?;
                        Ok(result)
                    })
            }
            Request::GitDiff(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.git_diff(&args, context)),
            Request::SqlStatus(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.sql_status(&args, context)),
            Request::SqlSearch(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.sql_search(&args, context)),
            Request::SqlMap(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.sql_map(&args, context)),
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
                let session = args.session.clone();
                if args.target_file.is_some() {
                    self.with_session_write(Some(&session), &mut lease, context, |session_id| {
                        let real_id = session_id.unwrap_or(&session);
                        let snapshot = self
                            .sessions
                            .lock()
                            .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                            .snapshot(real_id)?;
                        self.workspace.handover(&args, &snapshot, context)
                    })
                } else {
                    self.lease(Some(&session), &mut lease).and_then(|()| {
                        let real_id = lease.as_ref().map(|l| l.id.as_str()).unwrap_or(&session);
                        let snapshot = self
                            .sessions
                            .lock()
                            .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                            .snapshot(real_id)?;
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
            Request::PatchPreview(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.patch_preview(&args, context)),
            Request::Outline(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.outline(&args, context)),
            Request::ReadSymbol(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.read_symbol(&args, context)),
            Request::VueExtractPreview(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.vue_extract_preview(&args, context)),
            Request::Replace(args) => {
                if args.preview {
                    self.lease(args.session.as_deref(), &mut lease)
                        .and_then(|()| self.workspace.patch_preview(&args, context))
                } else {
                    self.with_session_write(
                        args.session.as_deref(),
                        &mut lease,
                        context,
                        |session_id| {
                            let mut result = self.workspace.replace(&args, context)?;
                            self.record_result(
                                session_id,
                                name,
                                &serde_json::to_value(&args).unwrap(),
                                &mut result,
                            );
                            Ok(result)
                        },
                    )
                }
            }
            Request::Write(args) => self.with_session_write(
                args.session.as_deref(),
                &mut lease,
                context,
                |session_id| {
                    let mut result = self.workspace.write(&args, context)?;
                    self.record_result(
                        session_id,
                        name,
                        &serde_json::to_value(&args).unwrap(),
                        &mut result,
                    );
                    Ok(result)
                },
            ),
            Request::Move(args) => self.with_session_write(
                args.session.as_deref(),
                &mut lease,
                context,
                |session_id| {
                    let mut result = self.workspace.move_file(&args, context)?;
                    self.record_result(
                        session_id,
                        name,
                        &serde_json::to_value(&args).unwrap(),
                        &mut result,
                    );
                    Ok(result)
                },
            ),
            Request::Read(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.read(&args, context)),
            Request::Search(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.search(&args, context)),
            Request::Ignore(args) => {
                if args.action == harness_domain::contracts::IgnoreAction::List {
                    self.lease(args.session.as_deref(), &mut lease)
                        .and_then(|()| self.workspace.ignore(&args, context))
                } else {
                    self.with_session_write(
                        args.session.as_deref(),
                        &mut lease,
                        context,
                        |session_id| {
                            let mut result = self.workspace.ignore(&args, context)?;
                            self.record_result(
                                session_id,
                                name,
                                &serde_json::to_value(&args).unwrap(),
                                &mut result,
                            );
                            Ok(result)
                        },
                    )
                }
            }
            Request::Verify(args) => self.with_session_write(
                args.session.as_deref(),
                &mut lease,
                context,
                |session_id| {
                    let mut result = self.workspace.verify(&args, context)?;
                    self.record_result(
                        session_id,
                        name,
                        &serde_json::to_value(&args).unwrap(),
                        &mut result,
                    );
                    Ok(result)
                },
            ),
            Request::Tasks(args) => {
                let session = args.session.clone();
                if matches!(
                    args.action,
                    Some(harness_domain::contracts::TasksAction::Update)
                ) {
                    self.with_session_write(Some(&session), &mut lease, context, |session_id| {
                        let real_id = session_id.unwrap_or(&session);
                        let update = self
                            .sessions
                            .lock()
                            .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                            .tasks(&args)?;
                        Ok(self.persist_update(real_id, update))
                    })
                } else {
                    self.lease(Some(&session), &mut lease).and_then(|()| {
                        self.sessions
                            .lock()
                            .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                            .tasks(&args)
                            .map(|update| update.data)
                    })
                }
            }
            Request::Checkpoint(args) => {
                let session = args.session.clone();
                if args.has_updates() || args.refresh_freshness == Some(true) {
                    self.with_session_write(Some(&session), &mut lease, context, |session_id| {
                        let real_id = session_id.unwrap_or(&session);
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
                            sessions.invalidate_checks(real_id, observation)?;
                        }
                        drop(sessions);
                        if let Some(observation) = observation {
                            harness_domain::freshness::refresh_checkpoint(
                                &mut update.data,
                                &observation,
                            );
                        }
                        Ok(self.persist_update(real_id, update))
                    })
                } else {
                    self.lease(Some(&session), &mut lease).and_then(|()| {
                        let mut sessions = self
                            .sessions
                            .lock()
                            .map_err(|_| ("INTERNAL", "Sessions unavailable"))?;
                        let update = sessions.checkpoint(&args)?;
                        drop(sessions);
                        Ok(update.data)
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
}
