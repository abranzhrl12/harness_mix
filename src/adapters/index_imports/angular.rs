//! Source-verified Angular token dependencies.
//!
//! Extraction metadata is advisory only. The binding is re-derived from the
//! current importer source before following the explicit module import.
use super::{ExportTarget, Failure, LIMIT, OperationContext, Project, Sources, module_path, sql};
use crate::adapters::index_semantic::RuntimeKind;
use rusqlite::Connection;

pub(in crate::adapters::index_store) struct AngularTarget {
    pub target: ExportTarget,
    pub module: String,
    pub imported: String,
    pub token_form: String,
}

impl Sources {
    /// Resolve an Angular route component only through a fresh unique named import.
    /// Local and project-wide same-name guesses are deliberately unsupported.
    pub(in crate::adapters::index_store) fn angular_route_component_target(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        from_node_id: &str,
        name: &str,
        reference_kind: &str,
        line: i64,
        column: i64,
    ) -> Result<Option<ExportTarget>, Failure> {
        if self
            .config
            .semantic
            .as_ref()
            .is_some_and(|policy| policy.excluded(path))
            || !self.observe(db, project, context, path)?
        {
            return Ok(None);
        }
        let source = std::str::from_utf8(self.verified.bytes(path).ok_or(super::INDEX)?)
            .map_err(|_| super::INDEX)?;
        if !crate::adapters::index_extract::angular::router::proves_reference(
            path,
            source,
            from_node_id,
            name,
            reference_kind,
            line,
            column,
            context,
        )? {
            return Ok(None);
        }
        let Some((module, imported)) =
            crate::adapters::index_extract::angular::router::named_runtime_import(
                path, source, name, context,
            )?
        else {
            return Ok(None);
        };
        let language: String = sql(db.query_row(
            "SELECT language FROM files WHERE path=?1",
            [path],
            |row| row.get(0),
        ))?;
        let Some(resolved) = module_path(db, path, &module, &language, context, self)? else {
            return Ok(None);
        };
        let Some(target) = self.runtime_export_target(
            db,
            project,
            context,
            &resolved,
            &imported,
            RuntimeKind::Class,
        )?
        else {
            return Ok(None);
        };
        if self
            .config
            .semantic
            .as_ref()
            .is_some_and(|policy| policy.excluded(&target.path))
        {
            return Ok(None);
        }
        Ok(Some(target))
    }

    pub(in crate::adapters::index_store) fn angular_token_target(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        name: &str,
        line: i64,
        col: i64,
    ) -> Result<Option<AngularTarget>, Failure> {
        if self
            .config
            .semantic
            .as_ref()
            .is_some_and(|policy| policy.excluded(path))
        {
            return Ok(None);
        }
        if !self.observe(db, project, context, path)? {
            return Ok(None);
        }
        if !self.angular.contains_key(path) {
            let source = std::str::from_utf8(self.verified.bytes(path).ok_or(super::INDEX)?)
                .map_err(|_| super::INDEX)?;
            let facts =
                crate::adapters::index_extract::angular::di::analyze(path, source, context)?;
            self.angular_reserved = self
                .angular_reserved
                .checked_add(facts.reserved)
                .ok_or(LIMIT)?;
            if self.angular_reserved > 16 * 1024 * 1024 {
                return Err(LIMIT);
            }
            self.angular.insert(path.into(), facts);
        }
        let Some(binding) = self
            .angular
            .get(path)
            .ok_or(super::INDEX)?
            .binding(name, line, col)
            .cloned()
        else {
            return Ok(None);
        };
        let language: String = sql(db.query_row(
            "SELECT language FROM files WHERE path=?1",
            [path],
            |row| row.get(0),
        ))?;
        if !matches!(
            language.as_str(),
            "typescript" | "tsx" | "javascript" | "jsx"
        ) {
            return Ok(None);
        }
        let Some(module) = module_path(db, path, &binding.module, &language, context, self)? else {
            return Ok(None);
        };
        let Some(target) = self.runtime_export_target(
            db,
            project,
            context,
            &module,
            &binding.imported,
            RuntimeKind::Class,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(AngularTarget {
            target,
            module: binding.module,
            imported: binding.imported,
            token_form: binding.token_form.as_str().to_owned(),
        }))
    }
}

/// Resolved framework refs are reconstituted from their original source data
/// before every framework-sensitive publication. This catches missing/new
/// modules, barrel changes, and newly ambiguous exported classes.
#[cfg(test)]
pub(in crate::adapters::index_store) fn invalidate(db: &Connection) -> Result<Vec<i64>, Failure> {
    let mut refs = super::invalidate_engine(db, "vue-named-composable-subset-v1")?;
    refs.extend(super::invalidate_engine(
        db,
        "angular-route-component-subset-v1",
    )?);
    refs.extend(super::invalidate_engine(
        db,
        "angular-token-dependency-subset-v1",
    )?);
    refs.extend(super::invalidate_engine(
        db,
        "angular-static-provider-registration-subset-v1",
    )?);
    refs.extend(super::invalidate_engine(
        db,
        "vue-composable-return-method-subset-v1",
    )?);
    refs.sort_unstable();
    refs.dedup();
    Ok(refs)
}
