//! Bounded ResolveExport adapter over Oxc's module records, not a new parser.
//! Star precedence, cycle and canonical-binding rules adapted from Rolldown
//! 1.2.14 link_stage/bind_imports_and_exports.rs and ECMAScript ResolveExport.
//! Copyright (c) 2024-present VoidZero Inc. & Contributors (MIT).
//! See ROLLDOWN-LICENSE.txt at the crate root for attribution/permission.
use super::{
    ExportTarget, Failure, INDEX, LIMIT, OperationContext, Project, Sources, hash, module_path, sql,
};
use crate::adapters::index_semantic::Export;
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};

#[derive(PartialEq, Eq)]
struct Binding {
    path: String,
    name: String,
}
enum Resolution {
    Missing,
    Bound(Binding),
    // Ambiguous, unsupported or unverified: never let another branch hide it.
    Unavailable,
}
struct Walk<'a> {
    sources: &'a mut Sources,
    db: &'a Connection,
    project: &'a Project,
    context: &'a OperationContext,
    active: BTreeSet<(String, String)>,
    contributors: BTreeMap<String, String>,
    steps: usize,
}

pub(super) fn resolve(
    sources: &mut Sources,
    db: &Connection,
    project: &Project,
    context: &OperationContext,
    path: &str,
    name: &str,
) -> Result<Option<ExportTarget>, Failure> {
    let mut walk = Walk {
        sources,
        db,
        project,
        context,
        active: BTreeSet::new(),
        contributors: BTreeMap::new(),
        steps: 0,
    };
    let Resolution::Bound(binding) = walk.visit(path, name)? else {
        return Ok(None);
    };
    Ok(Some(ExportTarget {
        path: binding.path,
        name: binding.name,
        contributors: walk
            .contributors
            .into_iter()
            .map(|(path, hash)| serde_json::json!({"path":path,"hash":hash}))
            .collect(),
    }))
}

impl Walk<'_> {
    fn visit(&mut self, path: &str, name: &str) -> Result<Resolution, Failure> {
        self.context.check()?;
        self.steps += 1;
        if self.steps > 256 {
            return Err(LIMIT);
        }
        // Default is not inherited by export-star; direct default is not enabled
        // in this native profile either. Never guess a same-named declaration.
        if name == "default" {
            return Ok(Resolution::Unavailable);
        }
        let key = (path.to_string(), name.to_string());
        if self.active.contains(&key) {
            return Ok(Resolution::Missing);
        }
        if self.active.len() >= 16 {
            return Err(LIMIT);
        }
        self.active.insert(key.clone());
        let result = self.entry(path, name);
        self.active.remove(&key);
        result
    }

    fn entry(&mut self, path: &str, name: &str) -> Result<Resolution, Failure> {
        if !self
            .sources
            .analyzed(self.db, self.project, self.context, path)?
        {
            return Ok(Resolution::Unavailable);
        }
        // Include unsuccessful branches too: adding an export there can turn a
        // previously unique binding into an ambiguity while both endpoints stay.
        self.contributors.insert(
            path.into(),
            hash(self.sources.files.get(path).ok_or(INDEX)?),
        );
        if self.contributors.len() > 16 {
            return Err(LIMIT);
        }
        let record = self.sources.semantic.get(path).ok_or(INDEX)?;
        if let Some(export) = record.exports.get(name).cloned() {
            return match export {
                Export::Local(local) => Ok(Resolution::Bound(Binding {
                    path: path.into(),
                    name: local,
                })),
                Export::Remote { module, name } => self.follow(path, &module, &name),
                Export::Unsupported => Ok(Resolution::Unavailable),
            };
        }
        let stars = record.stars.clone();
        let mut winner = None;
        for module in stars {
            match self.follow(path, &module, name)? {
                Resolution::Missing => {}
                Resolution::Unavailable => return Ok(Resolution::Unavailable),
                Resolution::Bound(binding) => {
                    // Compare original bindings, not paths through barrels:
                    // diamonds to the same function are not an ambiguity.
                    if winner.as_ref().is_some_and(|old| old != &binding) {
                        return Ok(Resolution::Unavailable);
                    }
                    winner = Some(binding);
                }
            }
        }
        Ok(winner.map_or(Resolution::Missing, Resolution::Bound))
    }

    fn follow(&mut self, from: &str, module: &str, name: &str) -> Result<Resolution, Failure> {
        let language: String =
            sql(self
                .db
                .query_row("SELECT language FROM files WHERE path=?1", [from], |r| {
                    r.get(0)
                }))?;
        if !matches!(
            language.as_str(),
            "typescript" | "tsx" | "javascript" | "jsx"
        ) {
            return Ok(Resolution::Unavailable);
        }
        let Some(target) =
            module_path(self.db, from, module, &language, self.context, self.sources)?
        else {
            return Ok(Resolution::Unavailable);
        };
        self.visit(&target, name)
    }
}
