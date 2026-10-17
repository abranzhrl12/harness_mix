//! Conservative JS/TS named-import scope. Uses indexed data, never probes paths.
use super::{Failure, INDEX, LIMIT, OperationContext, Project, hash, read_source, sql};
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Sources {
    files: BTreeMap<String, Vec<u8>>,
    reserved: usize,
    semantic: BTreeMap<String, super::super::index_semantic::Calls>,
    semantic_reserved: usize,
    routes: Option<super::paths::Routes>,
}
impl Sources {
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
        Ok(self
            .semantic
            .get(path)
            .ok_or(INDEX)?
            .contains(name, line, col))
    }
    pub(super) fn exported(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        language: &str,
        name: &str,
    ) -> Result<bool, Failure> {
        if !self.observe(db, project, context, path)? {
            return Ok(false);
        }
        let source = std::str::from_utf8(self.files.get(path).ok_or(INDEX)?).map_err(|_| INDEX)?;
        Ok(codegraph_native::named_function_export(
            source, language, name,
        ))
    }
    pub(super) fn recheck(
        &self,
        project: &Project,
        context: &OperationContext,
    ) -> Result<(), Failure> {
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
    let mut statement = sql(db.prepare("SELECT e.source,json_extract(e.metadata,'$.refName'),json_extract(e.metadata,'$.refKind'),e.line,e.col,s.file_path,s.language FROM edges e JOIN nodes s ON s.id=e.source WHERE CASE WHEN json_valid(e.metadata) THEN json_extract(e.metadata,'$.resolvedBy') END='native-relative-named-import-subset' LIMIT 20001"))?;
    let rows = sql(statement.query_map([], |r| {
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
    sql(db.execute("DELETE FROM edges WHERE CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.resolvedBy') END='native-relative-named-import-subset'", []))?;
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
    let Some(target) = relative_module(path, &module) else {
        return Ok(Scope::Pending);
    };
    if sources.routes.is_none() {
        sources.routes = Some(super::paths::Routes::load(db, context)?);
    }
    let Some(resolved) = sources
        .routes
        .as_mut()
        .ok_or(INDEX)?
        .resolve(path, &target, language, context)?
    else {
        return Ok(Scope::Pending);
    };
    Ok(Scope::Binding {
        path: resolved,
        name: original,
    })
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
mod tests {
    use super::*;
    #[test]
    fn module_normalization_never_escapes_workspace() {
        assert_eq!(
            relative_module("src/a.ts", "../lib/b"),
            Some("lib/b".into())
        );
        for module in [
            "../../outside",
            "pkg",
            "./x\\y",
            "./x?raw",
            "./%2e%2e/x",
            "./x:y",
        ] {
            assert_eq!(relative_module("src/a.ts", module), None);
        }
    }
    #[test]
    fn parses_named_aliases_but_defers_other_import_forms() {
        let (module, names) =
            codegraph_native::named_imports("import {run as execute, other} from './lib';")
                .unwrap();
        assert_eq!(module, "./lib");
        assert_eq!(
            names,
            [
                ("run".into(), "execute".into()),
                ("other".into(), "other".into())
            ]
        );
        for signature in [
            "import x from './lib'",
            "import * as x from './lib'",
            "import type { X } from './lib'",
            "import {type X} from './lib'",
            "import {x} from './lib'; evil()",
            "import {x} from './l\\u0069b'",
            "import {x} from './lib' with {type:'json'}",
        ] {
            assert!(
                codegraph_native::named_imports(signature).is_none(),
                "{signature}"
            );
        }
    }
}
