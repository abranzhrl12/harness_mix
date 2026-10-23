//! Native event-driven scope and storage capabilities.
use super::*;
use harness_application::ports::indexing::ChangedPathsPort;

pub(super) struct NativeChangedPaths<'a> {
    root: &'a Path,
    project: Project,
    scope: Option<scope::Snapshot>,
    scope_check_failed: bool,
    writer: Option<writer::Writer>,
}
impl<'a> NativeChangedPaths<'a> {
    pub(super) fn new(root: &'a Path) -> Result<Self, Failure> {
        Ok(Self {
            root,
            project: Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?,
            scope: None,
            scope_check_failed: false,
            writer: None,
        })
    }
    fn tracked(&self, path: &str) -> Result<bool, Failure> {
        sql(self.writer.as_ref().ok_or(INDEX)?.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE path=?1)",
            [path],
            |r| r.get(0),
        ))
    }
}
impl ChangedPathsPort for NativeChangedPaths<'_> {
    fn initialize(&mut self, context: &OperationContext) -> Result<(), Failure> {
        self.scope = Some(scope::Snapshot::load(&self.project, context)?);
        self.writer = Some(writer::Writer::open(&self.project, context)?);
        validate_schema(&self.writer.as_ref().ok_or(INDEX)?.db)
    }
    fn needs_inventory(&self, paths: &BTreeSet<String>) -> Result<bool, Failure> {
        if self.project.root.join(".git").is_file() {
            for path in paths {
                let tracked = self.tracked(path)?;
                if !tracked && path.split('/').any(index_inventory::excluded) {
                    continue;
                }
                if !tracked {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    fn sync_workspace(&mut self, context: &OperationContext) -> Result<Value, Failure> {
        self.writer.take();
        sync_workspace_all(self.root, context)
    }
    fn apply_changed(
        &mut self,
        path: &str,
        context: &OperationContext,
    ) -> Result<Option<Value>, Failure> {
        self.scope_check_failed = false;
        self.project.target_optional(path)?;
        let scope = self.scope.as_ref().ok_or(INDEX)?;
        if let Err(error) = scope.recheck(&self.project, context) {
            self.scope_check_failed = true;
            return Err(error);
        }
        if index_extract::language(path).is_none() || index_inventory::restricted(path) {
            return Ok(None);
        }
        let tracked = self.tracked(path)?;
        if !tracked && path.split('/').any(index_inventory::excluded) {
            return Ok(None);
        }
        if !tracked && !scope.included(path) && ignored_new_source(&self.project, path, context)? {
            return Ok(None);
        }
        sync_file_config_reuse(
            self.root,
            path,
            None,
            context,
            || {},
            false,
            &mut self.writer,
        )
        .map(Some)
    }
    fn file_failure_is_isolated(&self) -> bool {
        !self.scope_check_failed
    }
}
