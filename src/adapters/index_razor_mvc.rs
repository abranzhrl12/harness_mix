//! SQLite-backed Razor MVC route resolution, isolated from source parsing.
use super::parsers::razor::mvc::{Route, identifier, view_candidates, view_context};
use harness_domain::{Failure, execution::OperationContext};
use regex::Regex;
use rusqlite::{Connection, params};
use serde_json::Value;
use std::{collections::BTreeMap, sync::OnceLock};

fn regex(slot: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    slot.get_or_init(|| Regex::new(pattern).expect("constant MVC pattern"))
}

type Actions = BTreeMap<String, Option<String>>;
#[derive(Default)]
pub(in crate::adapters) struct Snapshot {
    controllers: BTreeMap<(String, String, String), Option<Actions>>,
    routes: BTreeMap<String, BTreeMap<String, Option<String>>>,
    views: BTreeMap<String, Option<String>>,
    route_count: usize,
    bytes: usize,
    #[cfg(test)]
    queries: usize,
    #[cfg(test)]
    parses: usize,
}

fn sql<T>(result: rusqlite::Result<T>) -> Result<T, Failure> {
    result.map_err(|_| ("INDEX_SQL", "Razor MVC index lookup failed"))
}

impl Snapshot {
    pub(in crate::adapters) fn resolve(
        &mut self,
        db: &Connection,
        name: &str,
        path: &str,
        context: &OperationContext,
    ) -> Result<Option<String>, Failure> {
        context.check()?;
        if let Some(target) = self.routes.get(path).and_then(|routes| routes.get(name)) {
            return Ok(target.clone());
        }
        if name.len() > 2048 || path.len() > 2048 {
            return Ok(None);
        }
        let Ok(route) = serde_json::from_str::<Route>(name) else {
            return Ok(None);
        };
        #[cfg(test)]
        {
            self.parses += 1;
        }
        let Some(view) = view_context(path) else {
            return Ok(None);
        };
        if route.root != view.root
            || !identifier(&route.controller)
            || !identifier(&route.action)
            || (!route.area.is_empty() && !identifier(&route.area))
        {
            return Ok(None);
        }
        let key = (
            if cfg!(windows) {
                route.root.to_ascii_lowercase()
            } else {
                route.root.clone()
            },
            route.area.to_ascii_lowercase(),
            route.controller.to_ascii_lowercase(),
        );
        if !self.controllers.contains_key(&key) {
            // A saturated publication defers rather than redoing uncached work.
            if self.controllers.len() >= 128 {
                return Ok(None);
            }
            let actions = self.load(db, &route, context)?;
            let bytes = 256
                + key.0.len()
                + key.1.len()
                + key.2.len()
                + actions.as_ref().map_or(0, |a| {
                    a.iter()
                        .map(|(name, id)| 128 + name.len() + id.as_ref().map_or(0, String::len))
                        .sum::<usize>()
                });
            if bytes > (4 * 1024 * 1024usize).saturating_sub(self.bytes) {
                return Ok(None);
            }
            self.bytes += bytes;
            self.controllers.insert(key.clone(), actions);
        }
        let target = self
            .controllers
            .get(&key)
            .and_then(Option::as_ref)
            .and_then(|actions| actions.get(&route.action.to_ascii_lowercase()))
            .and_then(Clone::clone);
        let bytes = 256 + path.len() + name.len() + target.as_ref().map_or(0, String::len);
        if self.route_count < 4096 && bytes <= (4 * 1024 * 1024usize).saturating_sub(self.bytes) {
            self.bytes += bytes;
            self.route_count += 1;
            self.routes
                .entry(path.to_owned())
                .or_default()
                .insert(name.to_owned(), target.clone());
        }
        Ok(target)
    }

    /// Resolve a literal partial or layout name to an indexed view file.
    /// Ambiguous or missing targets stay unresolved; nothing is guessed.
    pub(in crate::adapters) fn resolve_view(
        &mut self,
        db: &Connection,
        name: &str,
        path: &str,
        context: &OperationContext,
    ) -> Result<Option<String>, Failure> {
        let key = format!("{path}\u{0}{name}");
        if let Some(target) = self.views.get(&key) {
            return Ok(target.clone());
        }
        let mut found: Vec<String> = Vec::new();
        for candidate in view_candidates(path, name) {
            context.check()?;
            let mut statement = sql(db.prepare_cached(
                "SELECT id FROM nodes WHERE language='razor' AND kind='file' AND file_path=?1 COLLATE NOCASE LIMIT 2",
            ))?;
            let rows = sql(statement.query_map([&candidate], |row| row.get::<_, String>(0)))?;
            let rows = sql(rows.collect::<rusqlite::Result<Vec<_>>>())?;
            for row in rows {
                if !found.contains(&row) {
                    found.push(row);
                }
            }
        }
        // Two files with the same partial name are ambiguous: never choose.
        let target = if found.len() == 1 {
            Some(found[0].clone())
        } else {
            None
        };
        if self.views.len() < 4096 {
            self.views.insert(key, target.clone());
        }
        Ok(target)
    }

    fn load(
        &mut self,
        db: &Connection,
        route: &Route,
        context: &OperationContext,
    ) -> Result<Option<Actions>, Failure> {
        let controller = format!("{}Controller", route.controller);
        let prefix = if route.root.is_empty() {
            String::new()
        } else {
            format!("{}/", route.root)
        };
        let directory = if route.area.is_empty() {
            format!("{prefix}Controllers/")
        } else {
            format!("{prefix}Areas/{}/Controllers/", route.area)
        };
        let mut statement = sql(db.prepare_cached("SELECT qualified_name,file_path,decorators FROM nodes WHERE lower(name)=lower(?1) AND language='csharp' AND kind='class' AND visibility='public' AND coalesce(is_static,0)=0 AND coalesce(is_abstract,0)=0 AND (type_parameters IS NULL OR type_parameters IN ('[]','null')) LIMIT 65"))?;
        let rows = sql(statement.query_map([&controller], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        }))?;
        let rows = sql(rows.collect::<rusqlite::Result<Vec<_>>>())?;
        #[cfg(test)]
        {
            self.queries += 1;
        }
        if rows.len() >= 65 {
            return Ok(None);
        }
        let matches = rows
            .into_iter()
            .filter(|(_, path, decorators)| {
                let path = path.replace('\\', "/");
                let (path, directory) = if cfg!(windows) {
                    (path.to_ascii_lowercase(), directory.to_ascii_lowercase())
                } else {
                    (path, directory.clone())
                };
                path.strip_prefix(&directory)
                    .is_some_and(|tail| !tail.contains('/'))
                    && controller_attributes(decorators.as_deref(), &route.area)
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Ok(None);
        }
        let (owner, path, _) = &matches[0];
        let mut statement = sql(db.prepare_cached("SELECT id,name,qualified_name,decorators,signature FROM nodes WHERE file_path=?1 COLLATE BINARY AND language='csharp' AND kind='method' AND visibility='public' AND coalesce(is_static,0)=0 AND coalesce(is_abstract,0)=0 AND (type_parameters IS NULL OR type_parameters IN ('[]','null')) LIMIT 2001"))?;
        let rows = sql(statement.query_map(params![path], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        }))?;
        let rows = sql(rows.collect::<rusqlite::Result<Vec<_>>>())?;
        #[cfg(test)]
        {
            self.queries += 1;
        }
        if rows.len() > 2000 {
            return Ok(None);
        }
        let mut actions = Actions::new();
        for (id, name, qualified, decorators, signature) in rows {
            context.check()?;
            if qualified != format!("{owner}::{name}") {
                continue;
            }
            let Some(action) = action_name(&name, decorators.as_deref(), signature.as_deref())
            else {
                continue;
            };
            let entry = actions
                .entry(action.to_ascii_lowercase())
                .or_insert_with(|| Some(id.clone()));
            if entry.as_deref() != Some(&id) {
                *entry = None;
            }
        }
        Ok(Some(actions))
    }
}

fn attributes(raw: Option<&str>) -> String {
    fn strings(value: &Value, out: &mut String) {
        match value {
            Value::String(s) => {
                out.push_str(s);
                out.push(' ');
            }
            Value::Array(items) => {
                for item in items {
                    strings(item, out);
                }
            }
            Value::Object(_) => {
                out.push_str(&value.to_string());
            }
            _ => {}
        }
    }
    let raw = raw.unwrap_or("");
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        let mut out = String::new();
        strings(&value, &mut out);
        out
    } else {
        raw.to_owned()
    }
}

fn controller_attributes(raw: Option<&str>, area: &str) -> bool {
    let attrs = attributes(raw);
    if attrs.contains("NonController") {
        return false;
    }
    if !attrs.contains("Area") {
        return true;
    }
    static AREA: OnceLock<Regex> = OnceLock::new();
    let Some(capture) = regex(
        &AREA,
        r#"\bArea(?:Attribute)?\s*\(\s*"([A-Za-z0-9_]*)"\s*\)"#,
    )
    .captures(&attrs) else {
        return false;
    };
    capture[1].eq_ignore_ascii_case(area)
}

fn action_name(name: &str, raw: Option<&str>, signature: Option<&str>) -> Option<String> {
    let attrs = format!("{} {}", attributes(raw), signature.unwrap_or(""));
    if attrs.contains("NonAction") {
        return None;
    }
    if !attrs.contains("ActionName") {
        return Some(name.to_owned());
    }
    static ACTION: OnceLock<Regex> = OnceLock::new();
    let captures = regex(
        &ACTION,
        r#"\bActionName(?:Attribute)?\s*\(\s*"([A-Za-z0-9_]+)"\s*\)"#,
    )
    .captures_iter(&attrs)
    .collect::<Vec<_>>();
    let first = captures.first()?;
    captures
        .iter()
        .all(|capture| capture[1] == first[1])
        .then(|| first[1].to_owned())
}

#[cfg(test)]
#[path = "index_razor_mvc_tests.rs"]
mod tests;
