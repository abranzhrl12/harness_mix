//! Native workspace composition. Scheduling and report policy live in application.
use super::*;
use harness_application::ports::indexing::WorkspacePort;

pub(super) struct NativeWorkspace<'a> {
    root: &'a Path,
    project: Project,
    scope: Option<scope::Snapshot>,
    #[cfg(windows)]
    writer: Option<writer::Writer>,
}

impl<'a> NativeWorkspace<'a> {
    pub(super) fn new(root: &'a Path) -> Result<Self, Failure> {
        Ok(Self {
            root,
            project: Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?,
            scope: None,
            #[cfg(windows)]
            writer: None,
        })
    }
}

impl WorkspacePort for NativeWorkspace<'_> {
    type Prepared = parallel::Prepared;

    fn load_scope(&mut self, context: &OperationContext) -> Result<Value, Failure> {
        sync_diagnostic::begin(
            "<workspace-plan>",
            &self.project.root.join(".codegraph/codegraph.db"),
        );
        sync_diagnostic::stage("load_workspace_scope");
        let scope = scope::Snapshot::load(&self.project, context)?;
        let stamp = scope.stamp();
        self.scope = Some(scope);
        Ok(stamp)
    }
    fn plan(&mut self, context: &OperationContext) -> Result<Value, Failure> {
        sync_diagnostic::stage("plan_workspace");
        index_sync::plan_workspace_for_sync(self.root, context)
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
        sync_diagnostic::begin(path, &self.project.root.join(".codegraph/codegraph.db"));
        sync_diagnostic::stage("recheck_workspace_scope");
        self.recheck(context)?;
        #[cfg(windows)]
        {
            let invalidator: &publication::ReferenceInvalidator =
                if std::env::var("HARNESS_INDEX_BATCH_INVALIDATION").as_deref() == Ok("1") {
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
    fn recheck(&mut self, context: &OperationContext) -> Result<(), Failure> {
        self.scope
            .as_ref()
            .ok_or(INDEX)?
            .recheck(&self.project, context)
    }
}
