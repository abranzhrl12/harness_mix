//! Source-verified Vue composable calls. Extracted graph metadata is advisory;
//! every binding is re-derived from the current SFC before resolving its module.
use super::{ExportTarget, Failure, LIMIT, OperationContext, Project, Sources, module_path};
use crate::adapters::index_semantic::RuntimeKind;
use rusqlite::Connection;

impl Sources {
    pub(in crate::adapters::index_store) fn vue_call_target(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        name: &str,
        line: i64,
        col: i64,
    ) -> Result<Option<ExportTarget>, Failure> {
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
        if !self.vue.contains_key(path) {
            let source = std::str::from_utf8(self.files.get(path).ok_or(super::INDEX)?)
                .map_err(|_| super::INDEX)?;
            let facts = crate::adapters::index_semantic::vue::analyze(source, path, context)?;
            self.vue_reserved = self.vue_reserved.checked_add(facts.reserved).ok_or(LIMIT)?;
            if self.vue_reserved > 16 * 1024 * 1024 {
                return Err(LIMIT);
            }
            self.vue.insert(path.to_owned(), facts);
        }
        let Some((requested_module, imported, language)) = self
            .vue
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
        self.runtime_export_target(
            db,
            project,
            context,
            &module,
            &imported,
            RuntimeKind::Function,
        )
    }
}
