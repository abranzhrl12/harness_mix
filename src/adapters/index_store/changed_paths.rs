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

    fn sql_callers(db: &rusqlite::Connection) -> Result<Vec<String>, Failure> {
        let mut statement = sql(db.prepare(
            "SELECT file_path FROM (
                SELECT DISTINCT n.file_path AS file_path
                FROM edges e JOIN nodes n ON n.id=e.source
                WHERE CASE WHEN json_valid(e.metadata) THEN
                    json_extract(e.metadata,'$.resolvedBy')='local-sql-literal-v1'
                    AND json_extract(e.metadata,'$.refKind')='stored_procedure_call'
                    ELSE 0 END
                    AND n.language='csharp' AND n.file_path IS NOT NULL
                UNION
                SELECT DISTINCT file_path
                FROM unresolved_refs
                WHERE language='csharp' AND reference_kind='stored_procedure_call'
                    AND status IN ('pending','failed') AND file_path IS NOT NULL
            ) ORDER BY file_path LIMIT 257",
        ))?;
        let paths = sql(sql(statement.query_map([], |row| row.get::<_, String>(0)))?
            .collect::<rusqlite::Result<Vec<_>>>())?;
        if paths.len() > 256 {
            return Err((
                "SYNC_LIMIT",
                "SQL change has more than 256 dependent C# files; no dependents were re-synced",
            ));
        }
        Ok(paths)
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
        let is_sql = Path::new(path)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("sql"));
        if (index_extract::language(path).is_none() && !is_sql)
            || index_inventory::restricted(path)
        {
            return Ok(None);
        }
        let tracked = self.tracked(path)?;
        if !tracked && path.split('/').any(index_inventory::excluded) {
            return Ok(None);
        }
        // New exclusions are skips; indexed files still need graph cleanup.
        if !tracked && scope.excluded(path) {
            return Ok(None);
        }
        if !tracked && !scope.included(path) && ignored_new_source(&self.project, path, context)? {
            return Ok(None);
        }
        if is_sql {
            // Materialize the complete dependent set before re-syncing: the first C#
            // publication may remove/replace SQL graph edges used by later callers.
            let callers = Self::sql_callers(&self.writer.as_ref().ok_or(INDEX)?.db)?;
            let mut changed = 0usize;
            let mut committed = Vec::new();
            let mut failures = Vec::new();
            let mut failed_count = 0usize;
            let mut stopped_early = false;
            for caller in &callers {
                let result = context.check().and_then(|()| sync_file_config_reuse(
                    self.root,
                    caller,
                    None,
                    context,
                    || {},
                    true,
                    &mut self.writer,
                ));
                match result {
                    Ok(result) => {
                        committed.push(caller.clone());
                        changed += usize::from(result["changed"] == true);
                    }
                    Err((code, message)) => {
                        failed_count += 1;
                        if failures.len() < 32 {
                            failures.push(json!({"path":caller,"code":code,"message":message}));
                        }
                        if harness_application::indexing::workspace::systemic_failure(code)
                            || context.check().is_err() {
                            stopped_early = true;
                            break;
                        }
                    }
                }
            }
            return Ok(Some(json!({
                "changed": changed > 0,
                "sqlPath": path,
                "sqlRefreshComplete": failures.is_empty() && !stopped_early,
                "dependentFilesProcessed": committed.len() + failed_count,
                "dependentFilesChanged": changed,
                "dependentFilesCommitted": committed,
                "dependentFilesFailed": failed_count,
                "dependentFailures": failures,
                "dependentFailureReportsOmitted": failed_count.saturating_sub(32),
                "unattemptedDependents": callers.len().saturating_sub(committed.len() + failed_count),
                "stoppedEarly": stopped_early,
            })));
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

#[cfg(test)]
#[path = "changed_paths_tests.rs"]
mod tests;
