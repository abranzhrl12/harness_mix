//! Fresh-source proof for Angular's static provider registration relationships.
use super::{ExportTarget, Failure, LIMIT, OperationContext, Project, Sources, module_path, sql};
use crate::adapters::index_semantic::RuntimeKind;
use rusqlite::Connection;

pub(in crate::adapters::index_store) struct AngularProviderTarget {
    pub target: ExportTarget,
    pub module: String,
    pub imported: String,
    pub relationship: String,
    pub registration_form: String,
}

impl Sources {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::adapters::index_store) fn angular_provider_target(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        name: &str,
        line: i64,
        col: i64,
        kind: &str,
    ) -> Result<Option<AngularProviderTarget>, Failure> {
        if self
            .config
            .semantic
            .as_ref()
            .is_some_and(|policy| policy.excluded(path))
        {
            return Ok(None);
        }
        if !matches!(
            kind,
            "angular_provider_token" | "angular_provider_implementation"
        ) || !self.observe(db, project, context, path)?
        {
            return Ok(None);
        }
        if !self.angular_providers.contains_key(path) {
            let source = std::str::from_utf8(self.files.get(path).ok_or(super::INDEX)?)
                .map_err(|_| super::INDEX)?;
            let facts =
                crate::adapters::index_extract::angular::providers::analyze(path, source, context)?;
            self.angular_providers_reserved = self
                .angular_providers_reserved
                .checked_add(facts.reserved)
                .ok_or(LIMIT)?;
            if self.angular_providers_reserved > 16 * 1024 * 1024 {
                return Err(LIMIT);
            }
            self.angular_providers.insert(path.into(), facts);
        }
        let Some(binding) = self
            .angular_providers
            .get(path)
            .ok_or(super::INDEX)?
            .binding(name, line, col, kind)
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
        Ok(Some(AngularProviderTarget {
            target,
            module: binding.module,
            imported: binding.imported,
            relationship: binding.relationship,
            registration_form: binding.registration_form,
        }))
    }
}
