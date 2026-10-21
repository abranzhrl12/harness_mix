//! Source-verified returned-object methods for Vue composables.
use super::{ExportTarget, Failure, LIMIT, OperationContext, Project, Sources, module_path};
use crate::adapters::index_semantic::{RuntimeKind, vue_returns};
use rusqlite::Connection;

pub(in crate::adapters::index_store) struct MethodTarget {
    pub(in crate::adapters::index_store) target: ExportTarget,
    pub(in crate::adapters::index_store) implementation: vue_returns::Implementation,
}

impl Sources {
    /// Resolve one Vue method reference through a named composable import and
    /// an unconditional object return. Facts are re-derived from observed bytes
    /// on each invocation; the publication caller owns edge caching/invalidation.
    pub(in crate::adapters::index_store) fn vue_returned_method_target(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        name: &str,
        line: i64,
        col: i64,
    ) -> Result<Option<MethodTarget>, Failure> {
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
        let source = std::str::from_utf8(self.files.get(path).ok_or(super::INDEX)?)
            .map_err(|_| super::INDEX)?;
        if !self.vue_returns.contains_key(path) {
            let facts = vue_returns::analyze_calls(source, path, context)?;
            self.vue_returns_reserved = self
                .vue_returns_reserved
                .checked_add(facts.reserved)
                .ok_or(LIMIT)?;
            if self.vue_returns_reserved > 16 * 1024 * 1024 {
                return Err(LIMIT);
            }
            self.vue_returns.insert(path.to_owned(), facts);
        }
        let Some((requested_module, imported, language, property)) = self
            .vue_returns
            .get(path)
            .ok_or(super::INDEX)?
            .binding(name, line, col)
        else {
            return Ok(None);
        };
        let Some(module) = module_path(db, path, &requested_module, &language, context, self)?
        else {
            return Ok(None);
        };
        let Some(target) = self.runtime_export_target(
            db,
            project,
            context,
            &module,
            &imported,
            RuntimeKind::Function,
        )?
        else {
            return Ok(None);
        };
        if !self.observe(db, project, context, &target.path)? {
            return Ok(None);
        }
        let target_source = std::str::from_utf8(self.files.get(&target.path).ok_or(super::INDEX)?)
            .map_err(|_| super::INDEX)?;
        let Some(implementation) = vue_returns::returned_method(
            target_source,
            &target.path,
            &target.name,
            &property,
            context,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(MethodTarget {
            target,
            implementation,
        }))
    }
}
