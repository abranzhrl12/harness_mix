//! Conservative JS/TS named-import scope. Uses indexed data, never probes paths.
use super::{Failure, INDEX, LIMIT, OperationContext, Project, hash, read_source, sql};
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeMap;

#[path = "index_imports/angular.rs"]
pub(super) mod angular;
#[path = "index_imports/angular_providers.rs"]
mod angular_providers;
#[path = "index_exports.rs"]
mod exports;
#[path = "index_imports/vue.rs"]
mod vue;
#[path = "index_imports/vue_returns.rs"]
mod vue_returns;

#[derive(Default)]
pub(super) struct Sources {
    files: BTreeMap<String, Vec<u8>>,
    reserved: usize,
    semantic: BTreeMap<String, super::super::index_semantic::Calls>,
    semantic_reserved: usize,
    vue: BTreeMap<String, crate::adapters::index_semantic::vue::Facts>,
    vue_reserved: usize,
    vue_returns: BTreeMap<String, crate::adapters::index_semantic::vue_returns::Facts>,
    vue_returns_reserved: usize,
    angular: BTreeMap<String, crate::adapters::index_extract::angular::di::Facts>,
    angular_reserved: usize,
    angular_providers: BTreeMap<String, crate::adapters::index_extract::angular::providers::Facts>,
    angular_providers_reserved: usize,
    runtime: BTreeMap<
        (String, String, crate::adapters::index_semantic::RuntimeKind),
        Option<ExportTarget>,
    >,
    runtime_reserved: usize,
    rust: BTreeMap<String, super::rust::Facts>,
    rust_reserved: usize,
    razor: BTreeMap<String, super::razor_receivers::Facts>,
    razor_reserved: usize,
    razor_targets: super::razor_receivers::Targets,
    routes: Option<super::paths::Routes>,
    pub(super) config: super::tsconfig::Snapshot,
}
#[derive(Clone)]
pub(super) struct ExportTarget {
    pub path: String,
    pub name: String,
    pub contributors: Vec<serde_json::Value>,
}
impl Sources {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn razor_instance_target(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        name: &str,
        line: i64,
        col: i64,
    ) -> Result<Option<(String, String)>, Failure> {
        if !self.observe(db, project, context, path)? {
            return Ok(None);
        }
        if !self.razor.contains_key(path) {
            let source =
                std::str::from_utf8(self.files.get(path).ok_or(INDEX)?).map_err(|_| INDEX)?;
            let facts = super::razor_receivers::analyze(source, context)?;
            self.razor_reserved += facts.reserved();
            if self.razor_reserved > 16 * 1024 * 1024 {
                return Err(LIMIT);
            }
            self.razor.insert(path.into(), facts);
        }
        self.razor_targets.resolve(
            db,
            self.razor.get(path).ok_or(INDEX)?,
            name,
            line,
            col,
            context,
        )
    }
    fn rust_rooted(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        depth: usize,
    ) -> Result<bool, Failure> {
        if depth > 8 {
            return Ok(false);
        }
        let Some((dir, file)) = path.rsplit_once('/') else {
            return Ok(false);
        };
        if matches!(file, "lib.rs" | "main.rs") {
            return Ok(true);
        }
        if file != "mod.rs" {
            return Ok(false);
        }
        let Some((parent_dir, module)) = dir.rsplit_once('/') else {
            return Ok(false);
        };
        let mut parents = Vec::new();
        for file in ["mod.rs", "lib.rs", "main.rs"] {
            let parent = format!("{parent_dir}/{file}");
            if self.rust_analyzed(db, project, context, &parent)?
                && self.rust[&parent].modules.contains(module)
            {
                parents.push(parent);
            }
        }
        if parents.len() != 1 {
            return Ok(false);
        }
        self.rust_rooted(db, project, context, &parents[0], depth + 1)
    }
    fn rust_analyzed(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
    ) -> Result<bool, Failure> {
        if self.rust.contains_key(path) {
            return Ok(true);
        }
        if !self.files.contains_key(path) && self.files.len() >= 64 {
            return Ok(false);
        }
        if !self.observe(db, project, context, path)? {
            return Ok(false);
        }
        let source = std::str::from_utf8(self.files.get(path).ok_or(INDEX)?).map_err(|_| INDEX)?;
        let facts = match super::rust::analyze(source, context) {
            Ok(facts) => facts,
            Err(("PARSER_LIMIT" | "SYNTAX_ERROR", _)) => super::rust::Facts::default(),
            Err(error) => return Err(error),
        };
        let reserved = facts.reserved();
        if self.rust_reserved + reserved > 16 * 1024 * 1024 {
            self.rust.insert(path.into(), super::rust::Facts::default());
        } else {
            self.rust_reserved += reserved;
            self.rust.insert(path.into(), facts);
        }
        Ok(true)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn rust_target(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        name: &str,
        line: i64,
        col: i64,
    ) -> Result<Option<String>, Failure> {
        if !self.rust_analyzed(db, project, context, path)? {
            return Ok(None);
        }
        let Some(parts) = self.rust[path]
            .call(name, line, col)
            .map(<[String]>::to_vec)
        else {
            return Ok(None);
        };
        let (target, symbol) = if parts.len() == 1 {
            (path.to_owned(), parts[0].clone())
        } else if parts.len() == 3 && parts[0] == "super" {
            // Only siblings declared by one indexed parent module. No disk
            // probing, guessed global name or arbitrary suffix matching.
            let Some((dir, file)) = path.rsplit_once('/') else {
                return Ok(None);
            };
            let Some(stem) = file.strip_suffix(".rs") else {
                return Ok(None);
            };
            if matches!(stem, "mod" | "lib" | "main") {
                return Ok(None);
            }
            let mut parents = Vec::new();
            for parent in [
                format!("{dir}/mod.rs"),
                format!("{dir}/lib.rs"),
                format!("{dir}/main.rs"),
            ] {
                if self.rust_analyzed(db, project, context, &parent)?
                    && self.rust[&parent].modules.contains(stem)
                    && self.rust[&parent].modules.contains(&parts[1])
                {
                    parents.push(parent);
                }
            }
            if parents.len() != 1 {
                return Ok(None);
            }
            if !self.rust_rooted(db, project, context, &parents[0], 0)? {
                return Ok(None);
            }
            let mut targets = Vec::new();
            for target in [
                format!("{dir}/{}.rs", parts[1]),
                format!("{dir}/{}/mod.rs", parts[1]),
            ] {
                if sql(db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM files WHERE path=?1)",
                    [&target],
                    |r| r.get::<_, bool>(0),
                ))? {
                    targets.push(target);
                }
            }
            if targets.len() != 1 {
                return Ok(None);
            }
            let target = targets.remove(0);
            if !self.rust_analyzed(db, project, context, &target)? {
                return Ok(None);
            }
            (target, parts[2].clone())
        } else {
            return Ok(None);
        };
        let Some(&(start, public)) = self.rust[&target].functions.get(&symbol) else {
            return Ok(None);
        };
        if target != path && !public {
            return Ok(None);
        }
        let mut statement = sql(db.prepare_cached("SELECT id FROM nodes WHERE file_path=?1 AND name=?2 AND start_line=?3 AND kind='function' AND language='rust' LIMIT 2"))?;
        let ids = sql(sql(
            statement.query_map(rusqlite::params![target, symbol, start], |r| {
                r.get::<_, String>(0)
            }),
        )?
        .collect::<Result<Vec<_>, _>>())?;
        Ok(if ids.len() == 1 {
            ids.into_iter().next()
        } else {
            None
        })
    }
    pub fn configured(config: super::tsconfig::Snapshot) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }
    fn observe(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
    ) -> Result<bool, Failure> {
        if !self.files.contains_key(path) {
            if self.files.len() >= 64 || self.reserved + 1024 * 1024 > 64 * 1024 * 1024 {
                return Err(LIMIT);
            }
            let Some(bytes) = read_source(project, path, context)? else {
                return Ok(false);
            };
            let digest: Option<String> = sql(db
                .query_row(
                    "SELECT content_hash FROM files WHERE path=?1",
                    [path],
                    |r| r.get(0),
                )
                .optional())?;
            if digest.as_deref() != Some(hash(&bytes).as_str()) {
                return Ok(false);
            }
            self.reserved += bytes.len();
            self.files.insert(path.into(), bytes);
        }
        Ok(true)
    }
    #[allow(clippy::too_many_arguments)] // Existing reference location + bounded source observer.
    pub(super) fn permits(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        name: &str,
        line: i64,
        col: i64,
    ) -> Result<bool, Failure> {
        if !self.analyzed(db, project, context, path)? {
            return Ok(false);
        }
        Ok(self
            .semantic
            .get(path)
            .ok_or(INDEX)?
            .contains(name, line, col))
    }
    fn analyzed(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
    ) -> Result<bool, Failure> {
        if self
            .config
            .semantic
            .as_ref()
            .is_some_and(|policy| policy.excluded(path))
        {
            return Ok(false);
        }
        if !self.observe(db, project, context, path)? {
            return Ok(false);
        }
        if !self.semantic.contains_key(path) {
            let source =
                std::str::from_utf8(self.files.get(path).ok_or(INDEX)?).map_err(|_| INDEX)?;
            let calls = super::super::index_semantic::analyze(source, path, context)?;
            self.semantic_reserved += calls.reserved;
            if self.semantic_reserved > 16 * 1024 * 1024 {
                return Err(LIMIT);
            }
            self.semantic.insert(path.into(), calls);
        }
        Ok(true)
    }
    pub(super) fn export_target(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        name: &str,
    ) -> Result<Option<ExportTarget>, Failure> {
        exports::resolve(self, db, project, context, path, name)
    }
    pub(super) fn runtime_export_target(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        name: &str,
        kind: crate::adapters::index_semantic::RuntimeKind,
    ) -> Result<Option<ExportTarget>, Failure> {
        context.check()?;
        let key = (path.to_owned(), name.to_owned(), kind);
        if let Some(cached) = self.runtime.get(&key) {
            return Ok(cached.clone());
        }
        let target = exports::resolve_runtime(self, db, project, context, path, name, kind)?;
        let reserved = path.len()
            + name.len()
            + 64
            + target.as_ref().map_or(0, |target| {
                target.path.len()
                    + target.name.len()
                    + target
                        .contributors
                        .iter()
                        .map(|value| value.to_string().len())
                        .sum::<usize>()
            });
        // Facts and node/configuration snapshots are immutable during resolution.
        // Both successful and negative answers are safe only within this publication.
        if self.runtime.len() < 128
            && self.runtime_reserved.saturating_add(reserved) <= 2 * 1024 * 1024
        {
            self.runtime_reserved += reserved;
            self.runtime.insert(key, target.clone());
        }
        Ok(target)
    }
    pub(super) fn recheck(
        &self,
        project: &Project,
        context: &OperationContext,
    ) -> Result<(), Failure> {
        self.config.recheck(project, context)?;
        for (path, bytes) in &self.files {
            if read_source(project, path, context)?
                .as_ref()
                .map(|b| hash(b))
                != Some(hash(bytes))
            {
                return Err((
                    "STALE",
                    "Import resolution source changed; transaction rolled back",
                ));
            }
        }
        Ok(())
    }
}

pub(super) enum Scope {
    None,
    LocalOnly,
    Pending,
    Binding { path: String, name: String },
}

/// Recreate original refs before retrying native import edges. A new module can
/// make a formerly unique path ambiguous even if it has no incoming edges yet.
pub(super) fn invalidate(db: &Connection) -> Result<Vec<i64>, Failure> {
    invalidate_engine(db, "native-relative-named-import-subset")
}
pub(super) fn invalidate_rust(db: &Connection) -> Result<Vec<i64>, Failure> {
    let mut refs = invalidate_engine(db, "syn-module-subset-v1")?;
    refs.extend(invalidate_engine(db, "ra-module-subset-v1")?);
    Ok(refs)
}
fn invalidate_engine(db: &Connection, engine: &str) -> Result<Vec<i64>, Failure> {
    let mut statement = sql(db.prepare("SELECT e.source,json_extract(e.metadata,'$.refName'),json_extract(e.metadata,'$.refKind'),e.line,e.col,s.file_path,s.language FROM edges e JOIN nodes s ON s.id=e.source WHERE CASE WHEN json_valid(e.metadata) THEN json_extract(e.metadata,'$.resolvedBy') END=?1 LIMIT 20001"))?;
    let rows = sql(statement.query_map([engine], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
        ))
    }))?;
    let mut original = Vec::new();
    let mut reserved = 0;
    for row in rows {
        let row = sql(row)?;
        reserved += row.0.len() + row.1.len() + row.2.len() + row.5.len() + row.6.len();
        if original.len() >= super::MAX_ROWS || reserved > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        original.push(row);
    }
    sql(db.execute("DELETE FROM edges WHERE CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.resolvedBy') END=?1", [engine]))?;
    original
        .into_iter()
        .map(|(source, name, kind, line, col, path, language)| {
            super::insert_ref(
                db,
                &source,
                &name,
                &kind,
                line,
                col,
                None,
                (&path, &language),
            )
        })
        .collect()
}

pub(super) fn scope(
    db: &Connection,
    path: &str,
    name: &str,
    language: &str,
    context: &OperationContext,
    sources: &mut Sources,
) -> Result<Scope, Failure> {
    let mut statement = sql(db.prepare("SELECT signature FROM nodes WHERE file_path=?1 AND kind='import' ORDER BY start_line,id LIMIT 257"))?;
    let rows = sql(statement.query_map([path], |r| r.get::<_, Option<String>>(0)))?;
    let mut count = 0;
    let mut reserved = 0;
    let mut bindings = Vec::new();
    for row in rows {
        count += 1;
        let Some(signature) = sql(row)? else {
            return Ok(Scope::Pending);
        };
        reserved += signature.len();
        if count > 256 || reserved > 1024 * 1024 {
            return Err(LIMIT);
        }
        let Some((module, names)) = codegraph_native::named_imports(&signature) else {
            return Ok(Scope::Pending);
        };
        for (original, local) in names {
            if local == name {
                bindings.push((module.clone(), original));
            }
        }
    }
    if count == 0 {
        return Ok(Scope::None);
    }
    if bindings.is_empty() {
        return Ok(Scope::LocalOnly);
    }
    if bindings.len() != 1 {
        return Ok(Scope::Pending);
    }
    let (module, original) = bindings.pop().ok_or(INDEX)?;
    let Some(resolved) = module_path(db, path, &module, language, context, sources)? else {
        return Ok(Scope::Pending);
    };
    Ok(Scope::Binding {
        path: resolved,
        name: original,
    })
}

fn module_path(
    db: &Connection,
    path: &str,
    module: &str,
    language: &str,
    context: &OperationContext,
    sources: &mut Sources,
) -> Result<Option<String>, Failure> {
    let target = relative_module(path, module);
    if target.is_none()
        && (module.starts_with('.')
            || module.starts_with('/')
            || module.contains(['\\', ':', '?', '#'])
            || module.split('/').any(|p| matches!(p, "" | "." | ".."))
            || module.chars().any(char::is_control))
    {
        return Ok(None);
    }
    if sources.routes.is_none() {
        sources.routes = Some(super::paths::Routes::load_config(
            db,
            context,
            Some(&sources.config),
        )?);
    }
    let routes = sources.routes.as_mut().ok_or(INDEX)?;
    let resolved = match target {
        Some(target) => routes.resolve(path, &target, language, context)?,
        None if sources.config.matches(module) => {
            routes.resolve_bare(path, module, language, context)?
        }
        None => None,
    };
    Ok(resolved)
}

fn relative_module(from: &str, module: &str) -> Option<String> {
    if !(module.starts_with("./") || module.starts_with("../"))
        || module.len() > 1000
        || module.contains(['\\', ':', '?', '#', '%'])
        || module.chars().any(char::is_control)
    {
        return None;
    }
    let mut parts = from.split('/').collect::<Vec<_>>();
    parts.pop()?;
    for part in module.split('/') {
        match part {
            "." => {}
            ".." => {
                parts.pop()?;
            }
            "" => return None,
            value => parts.push(value),
        }
    }
    let path = parts.join("/");
    if path.is_empty() || super::index_inventory::restricted(&path) {
        return None;
    }
    Some(path)
}

#[cfg(test)]
#[path = "index_imports/tests.rs"]
mod tests;
