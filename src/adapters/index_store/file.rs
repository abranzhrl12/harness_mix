//! Native source snapshot and transaction factory.
use super::*;
use harness_application::ports::indexing::FileIndexPort;

pub(super) struct NativeFile<'a, F> {
    pub root: &'a Path,
    pub relative: String,
    pub requested_config: Option<&'a str>,
    pub before_recheck: Option<F>,
    pub force: bool,
    pub writer: &'a mut Option<writer::Writer>,
    pub prepared: Option<parallel::Prepared>,
    pub invalidator: &'a publication::ReferenceInvalidator,
    pub project: Option<Project>,
    pub scope: Option<scope::Snapshot>,
    pub source: Option<Vec<u8>>,
    pub language: Option<&'static str>,
}

impl<F: FnOnce()> FileIndexPort for NativeFile<'_, F> {
    type Transaction<'a>
        = transaction::SqliteFileTransaction<'a, 'a, F>
    where
        Self: 'a;
    fn validate_request(&mut self, _: &OperationContext) -> Result<(), Failure> {
        index_extract::take_diagnostic();
        let relative = self.relative.as_str();
        if relative.is_empty()
            || relative.len() > 1000
            || relative.contains('\\')
            || relative.chars().any(char::is_control)
            || relative.split('/').any(|s| matches!(s, "" | "." | ".."))
            || index_inventory::restricted(relative)
        {
            return Err(("PATH", "Invalid or restricted synchronization source path"));
        }
        self.language = Some(index_extract::language(relative).ok_or((
            "SYNC_LANGUAGE",
            "No native extractor for this source language",
        ))?);
        self.project =
            Some(Project::new(self.root).map_err(|_| ("PATH", "Workspace unavailable"))?);
        Ok(())
    }
    fn load_scope(&mut self, context: &OperationContext) -> Result<(), Failure> {
        let project = self.project.as_ref().ok_or(INDEX)?;
        sync_diagnostic::begin(
            &self.relative,
            &project.root.join(".codegraph/codegraph.db"),
        );
        sync_diagnostic::stage("load_file_scope");
        self.scope = Some(scope::Snapshot::load(project, context)?);
        Ok(())
    }
    fn read_source(&mut self, context: &OperationContext) -> Result<(), Failure> {
        sync_diagnostic::stage("read_source");
        self.source = if self.scope.as_ref().ok_or(INDEX)?.excluded(&self.relative) {
            None
        } else {
            read_source(self.project.as_ref().ok_or(INDEX)?, &self.relative, context)?
        };
        Ok(())
    }
    fn begin(&mut self, context: &OperationContext) -> Result<Self::Transaction<'_>, Failure> {
        let project = self.project.as_ref().ok_or(INDEX)?;
        let reused = self.writer.is_some();
        if self.writer.is_none() {
            sync_diagnostic::stage("open_database");
            *self.writer = Some(writer::Writer::open(project, context)?);
        }
        let writer = self.writer.as_mut().ok_or(INDEX)?;
        sync_diagnostic::stage("recheck_database_handle");
        writer.recheck(context)?;
        let cancel = context.clone();
        sync_diagnostic::stage("configure_connection");
        sql(writer
            .db
            .progress_handler(1000, Some(move || cancel.check().is_err())))?;
        sql(writer.db.execute_batch("PRAGMA trusted_schema=OFF;"))?;
        sync_diagnostic::stage("begin_transaction");
        let tx = sql(writer
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate))?;
        Ok(transaction::SqliteFileTransaction {
            tx,
            project,
            scope: self.scope.as_ref().ok_or(INDEX)?,
            relative: &self.relative,
            language: self.language.ok_or(INDEX)?,
            source: self.source.as_deref(),
            requested_config: self.requested_config,
            force: self.force,
            prepared: self.prepared.take(),
            retry_memo: &mut writer.retry_memo,
            rust_analysis_cache: &writer.rust_analysis_cache,
            invalidator: self.invalidator,
            before_recheck: self.before_recheck.take(),
            reused,
        })
    }
}
