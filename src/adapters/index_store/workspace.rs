//! Native workspace composition. Scheduling and report policy live in application.
use super::*;
use harness_application::ports::indexing::WorkspacePort;

pub(super) struct NativeWorkspace<'a> {
    root: &'a Path,
    project: Project,
    scope: Option<scope::Snapshot>,
    scope_check_failed: bool,
    #[cfg(windows)]
    writer: Option<writer::Writer>,
}

impl<'a> NativeWorkspace<'a> {
    pub(super) fn assess_archive(&mut self, report: &mut Value, context: &OperationContext) {
        #[cfg(windows)]
        {
            let assessment = (|| {
                context.check()?;
                if self.writer.is_none() {
                    self.writer = Some(writer::Writer::open(&self.project, context)?);
                }
                quarantine::coverage(&self.writer.as_ref().ok_or(INDEX)?.db, None, None)
            })();
            match assessment {
                Ok(coverage) => {
                    quarantine::annotate(report, &coverage, false);
                    if coverage.pending > 0 {
                        report["selectedScopeApplied"] = json!(false);
                    }
                }
                Err((code, message)) => {
                    // Preserve confirmed per-file commits even if the final
                    // read fails. Unknown coverage must not become success.
                    report["selectedScopeApplied"] = json!(false);
                    report["graphStatus"] = json!("unknown");
                    report["archiveAssessment"] = json!({"code":code,"message":message});
                }
            }
        }
        #[cfg(not(windows))]
        let _ = (report, context);
    }

    pub(super) fn new(root: &'a Path) -> Result<Self, Failure> {
        Ok(Self {
            root,
            project: Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?,
            scope: None,
            scope_check_failed: false,
            #[cfg(windows)]
            writer: None,
        })
    }

    #[cfg(windows)]
    fn persist_failed_rebuild(
        &mut self,
        path: &str,
        context: &OperationContext,
    ) -> Result<bool, Failure> {
        context.check()?;
        let writer = self.writer.as_mut().ok_or(INDEX)?;
        writer.recheck(context)?;
        sync_diagnostic::stage("mark_failed_rebuild");
        const FAILURE_MARKER: &str = r#"["SYNC_REBUILD_FAILED"]"#;
        let updated = sql(writer.db.execute(
            "UPDATE files SET errors=?1 WHERE path=?2",
            [FAILURE_MARKER, path],
        ))?;
        Ok(updated > 0)
    }

    /// One transaction retries every still-pending reference against the final
    /// graph. A failure rolls back and leaves the refs pending for a later sync.
    #[cfg(windows)]
    fn resolve_deferred_references(
        &mut self,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        context.check()?;
        let started = std::time::Instant::now();
        let writer = self.writer.as_mut().ok_or(INDEX)?;
        writer.recheck(context)?;
        let cancel = context.clone();
        sync_diagnostic::stage("final_resolution");
        sql(writer
            .db
            .progress_handler(1000, Some(move || cancel.check().is_err())))?;
        sql(writer.db.execute_batch("PRAGMA trusted_schema=OFF;"))?;
        let tx = sql(writer
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate))?;
        validate_schema(&tx)?;
        sql(tx.execute_batch("PRAGMA trusted_schema=ON"))?;
        let (configuration, _) = tsconfig::Snapshot::select(&tx, &self.project, None, context)?;
        let ids: Vec<i64> = {
            let mut statement = sql(tx.prepare(
                "SELECT id FROM unresolved_refs WHERE status='pending' ORDER BY id",
            ))?;
            let rows = sql(statement.query_map([], |row| row.get(0)))?;
            sql(rows.collect())?
        };
        let mut sources =
            imports::Sources::with_rust_cache(configuration, writer.rust_analysis_cache.clone());
        let mut candidate_snapshot = candidates::Snapshot::default();
        let mut mvc_snapshot = crate::adapters::index_razor_mvc::Snapshot::default();
        let mut framework_snapshot = repository::framework::Snapshot::default();
        let mut resolved = 0usize;
        for id in &ids {
            context.check()?;
            resolved += usize::from(resolution::resolve_exact(
                &tx,
                *id,
                &self.project,
                context,
                &mut sources,
                &mut candidate_snapshot,
                &mut mvc_snapshot,
                &mut framework_snapshot,
            )?);
        }
        framework_snapshot.recheck(&self.project, context)?;
        self.scope
            .as_ref()
            .ok_or(INDEX)?
            .recheck(&self.project, context)?;
        sql(tx.progress_handler(0, None::<fn() -> bool>))?;
        tx.commit().map_err(|_| {
            (
                "SYNC_COMMIT_UNCERTAIN",
                "Final resolution commit could not be confirmed; inspect the index before retrying",
            )
        })?;
        Ok(json!({"committed":true,"referencesAttempted":ids.len(),
            "referencesResolved":resolved,"durationMs":started.elapsed().as_millis()}))
    }
}

/// Rebuilds publish files without per-file cross-file retries and resolve them
/// once at the end. `HARNESS_INDEX_BATCH_INVALIDATION=0` restores per-file retries.
fn batch_rebuild_enabled() -> bool {
    std::env::var("HARNESS_INDEX_BATCH_INVALIDATION").as_deref() != Ok("0")
}

impl WorkspacePort for NativeWorkspace<'_> {
    type Prepared = parallel::Prepared;

    fn load_scope(&mut self, context: &OperationContext) -> Result<Value, Failure> {
        sync_diagnostic::begin(
            "<workspace-plan>",
            &self.project.root.join(self.project.index_rel("codegraph.db")),
        );
        sync_diagnostic::stage("load_workspace_scope");
        let scope = scope::Snapshot::load(&self.project, context)?;
        let stamp = scope.stamp();
        self.scope = Some(scope);
        Ok(stamp)
    }
    fn plan(&mut self, context: &OperationContext) -> Result<Value, Failure> {
        sync_diagnostic::stage("plan_workspace");
        let mut plan = index_sync::plan_workspace_for_sync(self.root, context)?;
        #[cfg(windows)]
        {
            context.check()?;
            if self.writer.is_none() {
                self.writer = Some(writer::Writer::open(&self.project, context)?);
            }
            let db = &self.writer.as_ref().ok_or(INDEX)?.db;
            validate_schema(db)?;
            let outdated = versioning::outdated(db)?;
            let unchanged = plan["plan"]["unchanged"].as_array_mut().ok_or(INDEX)?;
            let mut refresh = Vec::new();
            unchanged.retain(|path| {
                if path.as_str().is_some_and(|path| outdated.contains(path)) {
                    refresh.push(path.clone());
                    false
                } else {
                    true
                }
            });
            plan["plan"]["retryExtraction"]
                .as_array_mut()
                .ok_or(INDEX)?
                .extend(refresh);
        }
        Ok(plan)
    }
    fn language(&self, path: &str) -> Option<&'static str> {
        index_extract::language(path)
    }
    fn workers(&self) -> usize {
        parallel::workers()
    }
    fn prepare(
        &mut self,
        paths: &[&String],
        context: &OperationContext,
    ) -> Vec<Option<Self::Prepared>> {
        parallel::prepare(self.root, paths, context)
    }
    fn apply(
        &mut self,
        path: &str,
        prepared: Option<Self::Prepared>,
        force: bool,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        sync_diagnostic::begin(path, &self.project.root.join(self.project.index_rel("codegraph.db")));
        sync_diagnostic::stage("recheck_workspace_scope");
        self.scope_check_failed = false;
        if let Err(error) = self.recheck(context) {
            self.scope_check_failed = true;
            return Err(error);
        }
        #[cfg(windows)]
        {
            let invalidator: &publication::ReferenceInvalidator =
                if force && batch_rebuild_enabled() {
                    &publication::BatchSqliteInvalidator
                } else {
                    &publication::IncrementalSqliteInvalidator
                };
            sync_file_config_prepared(
                self.root,
                path,
                None,
                context,
                || {},
                force,
                &mut self.writer,
                prepared,
                invalidator,
            )
        }
        #[cfg(not(windows))]
        {
            let _ = (prepared, force);
            sync_file(self.root, path, context)
        }
    }
    fn diagnostic(&mut self, code: &str, message: &str) -> Option<Value> {
        sync_diagnostic::take(code, message)
    }
    fn file_failure_is_isolated(&self) -> bool {
        !self.scope_check_failed
    }
    #[cfg(windows)]
    fn mark_failed_rebuild(
        &mut self,
        path: &str,
        context: &OperationContext,
    ) -> Result<bool, Failure> {
        self.persist_failed_rebuild(path, context)
    }
    fn finalize_rebuild(&mut self, context: &OperationContext) -> Result<Option<Value>, Failure> {
        #[cfg(windows)]
        {
            if !batch_rebuild_enabled() {
                return Ok(None);
            }
            self.resolve_deferred_references(context).map(Some)
        }
        #[cfg(not(windows))]
        {
            let _ = context;
            Ok(None)
        }
    }
    fn recheck(&mut self, context: &OperationContext) -> Result<(), Failure> {
        self.scope
            .as_ref()
            .ok_or(INDEX)?
            .recheck(&self.project, context)
    }
}
