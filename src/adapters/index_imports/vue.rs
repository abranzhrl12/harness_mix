//! Source-verified Vue composable calls and route component resolution.
//! Extracted graph metadata is advisory; every binding is re-derived from the
//! current SFC/module before resolving its target.
use super::{ExportTarget, Failure, LIMIT, OperationContext, Project, Sources, module_path};
use crate::adapters::index_semantic::RuntimeKind;
use crate::adapters::index_store::sql;
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
            let source = std::str::from_utf8(self.verified.bytes(path).ok_or(super::INDEX)?)
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

    pub(in crate::adapters::index_store) fn vue_route_component_target(
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
    ) -> Result<Option<String>, Failure> {
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
        if !crate::adapters::index_extract::vue_router::proves_reference(
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
        let module_specifier = if reference_kind == "vue_route_dynamic_component" {
            name.to_string()
        } else {
            let Some((module, _)) =
                crate::adapters::index_extract::vue_router::named_runtime_import(
                    path, source, name, context,
                )?
            else {
                return Ok(None);
            };
            module
        };
        let language: String = sql(db.query_row(
            "SELECT language FROM files WHERE path=?1",
            [path],
            |row| row.get(0),
        ))?;
        let Some(resolved) = module_path(db, path, &module_specifier, &language, context, self)? else {
            return Ok(None);
        };
        if self
            .config
            .semantic
            .as_ref()
            .is_some_and(|policy| policy.excluded(&resolved))
        {
            return Ok(None);
        }
        Ok(Some(resolved))
    }

    pub(in crate::adapters::index_store) fn vue_template_component_target(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        from_node_id: &str,
        name: &str,
        line: i64,
        column: i64,
    ) -> Result<Option<(String, String)>, Failure> {
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
        if !self.vue_template_components.contains_key(path) {
            let components = crate::adapters::index_extract::template_component_imports(
                path, source, context,
            )?;
            let mut reserved = path.len().checked_add(128).ok_or(LIMIT)?;
            for (id, (component, _, _, module)) in &components {
                let item_bytes = id
                    .len()
                    .checked_add(component.len())
                    .and_then(|bytes| bytes.checked_add(module.len()))
                    .and_then(|bytes| bytes.checked_add(64))
                    .ok_or(LIMIT)?;
                reserved = reserved
                    .checked_add(item_bytes)
                    .ok_or(LIMIT)?;
            }
            self.vue_template_components_reserved = self
                .vue_template_components_reserved
                .checked_add(reserved)
                .ok_or(LIMIT)?;
            if self.vue_template_components_reserved > 16 * 1024 * 1024 {
                return Err(LIMIT);
            }
            self.vue_template_components
                .insert(path.to_owned(), components);
        }
        let Some((proved_name, proved_line, proved_column, module_specifier)) = self
            .vue_template_components
            .get(path)
            .ok_or(LIMIT)?
            .get(from_node_id)
        else {
            return Ok(None);
        };
        if proved_name != name || *proved_line != line || *proved_column != column {
            return Ok(None);
        }
        let module_specifier = module_specifier.clone();
        let imported = "default";
        let Some(resolved) =
            module_path(db, path, &module_specifier, "vue", context, self)?
        else {
            return Ok(None);
        };
        if !resolved.to_ascii_lowercase().ends_with(".vue")
            || self
                .config
                .semantic
                .as_ref()
                .is_some_and(|policy| policy.excluded(&resolved))
            || !self.observe(db, project, context, &resolved)?
        {
            return Ok(None);
        }
        Ok(Some((resolved, imported.to_string())))
    }
}
