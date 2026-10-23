//! SQLite transaction mechanics; lifecycle decisions belong to application.
use super::*;
use harness_application::ports::indexing::FileTransactionPort;

pub(super) struct SqliteFileTransaction<'a, 'db, F> {
    pub tx: rusqlite::Transaction<'db>,
    pub project: &'a Project,
    pub scope: &'a scope::Snapshot,
    pub relative: &'a str,
    pub language: &'a str,
    pub source: Option<&'a [u8]>,
    pub requested_config: Option<&'a str>,
    pub force: bool,
    pub prepared: Option<parallel::Prepared>,
    pub retry_memo: &'a mut resolution::RetryMemo,
    pub rust_analysis_cache: &'a imports::RustFactsCache,
    pub invalidator: &'a publication::ReferenceInvalidator,
    pub before_recheck: Option<F>,
    pub reused: bool,
}

impl<F: FnOnce()> FileTransactionPort for SqliteFileTransaction<'_, '_, F> {
    type Publication = (Value, imports::Sources);
    type Output = Value;

    fn validate(&mut self, _: &OperationContext) -> Result<(), Failure> {
        let data_version: i64 = sql(self
            .tx
            .query_row("PRAGMA data_version", [], |row| row.get(0)))?;
        self.retry_memo.observe_version(data_version);
        sync_diagnostic::stage("validate_schema");
        validate_schema(&self.tx)?;
        // Trusted FTS triggers are enabled only after exact schema validation.
        sql(self.tx.execute_batch("PRAGMA trusted_schema=ON"))
    }
    fn publish(&mut self, context: &OperationContext) -> Result<Self::Publication, Failure> {
        sync_diagnostic::stage("select_configuration");
        let configuration =
            tsconfig::Snapshot::select(&self.tx, self.project, self.requested_config, context)?;
        let (mut result, sources) = publish(
            &self.tx,
            self.project,
            self.relative,
            self.language,
            self.source,
            context,
            configuration,
            self.force,
            self.prepared.take(),
            self.retry_memo,
            self.rust_analysis_cache,
            self.invalidator,
        )?;
        result["tsconfig"] = sources.config.stamp();
        Ok((result, sources))
    }
    fn recheck(
        &mut self,
        publication: &Self::Publication,
        context: &OperationContext,
    ) -> Result<(), Failure> {
        sync_diagnostic::stage("recheck_sources");
        if let Some(before) = self.before_recheck.take() {
            before();
        }
        {
            let _timer = sync_diagnostic::operation("recheck_scope");
            self.scope.recheck(self.project, context)?;
        }
        {
            let _timer = sync_diagnostic::operation("recheck_dependencies");
            publication.1.recheck(self.project, context)?;
        }
        let _timer = sync_diagnostic::operation("recheck_primary");
        let observed = if self.scope.excluded(self.relative) {
            None
        } else {
            read_source(self.project, self.relative, context)?
        };
        if observed.as_ref().map(|b| hash(b)) != self.source.map(hash) {
            return Err((
                "STALE",
                "Source changed before index publication; transaction rolled back",
            ));
        }
        Ok(())
    }
    fn integrity(
        &mut self,
        publication: &Self::Publication,
        _: &OperationContext,
    ) -> Result<(), Failure> {
        sync_diagnostic::stage("integrity_check");
        let violations: bool = publication.0["changed"] == true && sql(self.tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM edges e LEFT JOIN nodes s ON s.id=e.source LEFT JOIN nodes t ON t.id=e.target WHERE (e.source IN(SELECT id FROM nodes WHERE file_path=?1) OR e.target IN(SELECT id FROM nodes WHERE file_path=?1)) AND (s.id IS NULL OR t.id IS NULL))",
            [self.relative], |row| row.get(0)))?;
        if violations {
            return Err(("SYNC_INTEGRITY", "Index foreign-key integrity check failed"));
        }
        Ok(())
    }
    fn rollback(self) -> Result<(), Failure> {
        self.retry_memo.clear();
        self.tx
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(|_| UNCERTAIN)?;
        self.tx.rollback().map_err(|_| UNCERTAIN)
    }
    fn commit(self, (mut result, _): Self::Publication) -> Result<Value, Failure> {
        self.tx
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(|_| UNCERTAIN)?;
        sync_diagnostic::stage("commit_transaction");
        sync_diagnostic::count("commit_attempts", 1);
        self.tx.commit().map_err(|error| {
            self.retry_memo.clear();
            sync_diagnostic::capture(&error);
            UNCERTAIN
        })?;
        result["sourceSnapshotAtomic"] = json!(false);
        result["writerConnectionReused"] = json!(self.reused);
        result["committed"] = json!(true);
        if let Some(profile) = sync_diagnostic::finish_profile() {
            result["phaseTimingsMicros"] = profile;
            result["profileMetrics"] = sync_diagnostic::profile_metrics().unwrap_or(Value::Null);
        }
        Ok(result)
    }
}
