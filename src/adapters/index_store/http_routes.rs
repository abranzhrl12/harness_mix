//! Literal HTTP links reuse calls edges and the existing reference schema.
use super::repository::framework::Snapshot;
use super::{INDEX, LIMIT, MAX_ROWS, repository, sql};
use crate::adapters::project::Project;
use harness_domain::{Failure, execution::OperationContext, hash};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Catalog {
    routes: BTreeMap<String, Vec<Target>>,
    verified: BTreeSet<String>,
}
struct Target {
    id: String,
    path: String,
    digest: Option<String>,
    method: Option<String>,
}

/// Deliberately local paths only: a URL on another host is not a local caller.
fn normalize(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.contains("://")
        || raw.starts_with("//")
        || raw.contains(['\\', ':', '{', '}'])
        || raw.chars().any(char::is_control)
    {
        return None;
    }
    let path = raw.split(['?', '#']).next()?.trim_matches('/');
    if path.is_empty() || path.split('/').any(|part| matches!(part, "." | ".." | "")) {
        return None;
    }
    Some(path.to_lowercase())
}

impl Catalog {
    fn load(db: &Connection, context: &OperationContext) -> Result<Self, Failure> {
        let mut catalog = Self::default();
        let mut statement = sql(db.prepare(
            "SELECT n.id,n.file_path,f.content_hash,substr(d.value,14) FROM nodes n
             LEFT JOIN files f ON f.path=n.file_path,
             json_each(CASE WHEN json_valid(n.decorators) THEN n.decorators ELSE '[]' END) d
             WHERE n.language='csharp' AND n.kind='method' AND d.type='text'
             AND substr(d.value,1,13)='harness:http:' LIMIT 20001",
        ))?;
        let rows = sql(statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        }))?;
        let mut reserved = 0usize;
        for (count, row) in rows.enumerate() {
            context.check()?;
            let (id, path, digest, route) = sql(row)?;
            reserved +=
                id.len() + path.len() + route.len() + digest.as_ref().map_or(0, String::len);
            if count >= MAX_ROWS || reserved > 16 * 1024 * 1024 {
                return Err(LIMIT);
            }
            let route: Value = serde_json::from_str(&route).map_err(|_| INDEX)?;
            let Some(normalized) = route["path"].as_str().and_then(normalize) else {
                continue;
            };
            catalog.routes.entry(normalized).or_default().push(Target {
                id,
                path,
                digest,
                method: route["method"].as_str().map(str::to_ascii_uppercase),
            });
        }
        Ok(catalog)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve(
    db: &Connection,
    id: i64,
    from: &str,
    name: &str,
    line: i64,
    col: i64,
    path: &str,
    project: &Project,
    context: &OperationContext,
    snapshot: &mut Snapshot,
) -> Result<bool, Failure> {
    let request: Value = serde_json::from_str(name).map_err(|_| INDEX)?;
    let Some(route) = request["url"].as_str().and_then(normalize) else {
        return Ok(false);
    };
    let Some(method) = request["method"].as_str() else {
        return Ok(false);
    };
    if snapshot.http_routes.is_none() {
        snapshot.http_routes = Some(Catalog::load(db, context)?);
    }
    let catalog = snapshot.http_routes.as_ref().ok_or(INDEX)?;
    let Some(targets) = catalog.routes.get(&route) else {
        return Ok(false);
    };
    let matching: Vec<_> = targets
        .iter()
        .filter(|target| {
            target
                .method
                .as_ref()
                .is_none_or(|verb| verb.eq_ignore_ascii_case(method))
        })
        .collect();
    let unique: BTreeSet<_> = matching.iter().map(|target| target.id.as_str()).collect();
    if unique.len() != 1 {
        return Ok(false);
    }
    let target = matching[0];
    let (target_id, target_path, target_digest) = (
        target.id.clone(),
        target.path.clone(),
        target.digest.clone(),
    );
    let Some(target_digest) = target_digest else {
        return Ok(false);
    };
    let source_digest: Option<String> = sql(db
        .query_row(
            "SELECT content_hash FROM files WHERE path=?1",
            [path],
            |row| row.get(0),
        )
        .optional())?;
    let Some(source_digest) = source_digest else {
        return Ok(false);
    };
    for (file, digest) in [
        (path, source_digest.as_str()),
        (target_path.as_str(), target_digest.as_str()),
    ] {
        if !snapshot
            .http_routes
            .as_ref()
            .ok_or(INDEX)?
            .verified
            .contains(file)
        {
            let current = match super::read_source(project, file, context) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => return Ok(false),
                Err(error @ ("CANCELLED" | "TIMEOUT", _)) => return Err(error),
                Err(_) => return Ok(false),
            };
            if hash(&current) != digest {
                return Ok(false);
            }
            snapshot.remember(file, digest)?;
            snapshot
                .http_routes
                .as_mut()
                .ok_or(INDEX)?
                .verified
                .insert(file.to_owned());
        }
    }
    let confidence = request["confidence"].as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= 0.9)
        .unwrap_or(0.9);
    let metadata = json!({
        "transport":"http", "refKind":"http_route", "refName":name,
        "url":request["url"], "httpMethod":method.to_ascii_uppercase(),
        "route":route, "resolvedBy":"native-http-literal-v1",
        "sourceHash":source_digest, "targetHash":target_digest,
        "extraction":request.get("extraction"), "confidence":confidence
    })
    .to_string();
    sql(db.execute(
        "INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,'calls',?3,?4,?5,'heuristic')",
        params![from,target_id,line,col,metadata],
    ))?;
    sql(db.execute("DELETE FROM unresolved_refs WHERE id=?1", [id]))?;
    Ok(true)
}

/// A backend edit can change route text or introduce ambiguity without changing
/// method identity. Recover all HTTP sites before capture, inside the transaction.
pub(super) fn invalidate(db: &Connection, context: &OperationContext) -> Result<(), Failure> {
    let mut statement = sql(db.prepare(
        "SELECT e.source,e.line,e.col,e.metadata,n.file_path,n.language FROM edges e
         JOIN nodes n ON n.id=e.source
         WHERE CASE WHEN json_valid(e.metadata) THEN json_extract(e.metadata,'$.refKind') END='http_route'
         LIMIT 20001",
    ))?;
    let rows = sql(statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<i64>>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    }))?;
    let mut sites = Vec::new();
    let mut reserved = 0usize;
    for row in rows {
        context.check()?;
        let site = sql(row)?;
        reserved += site.0.len() + site.3.len() + site.4.len() + site.5.len();
        if sites.len() >= MAX_ROWS || reserved > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        sites.push(site);
    }
    drop(statement);
    for (from, line, col, metadata, path, language) in sites {
        context.check()?;
        let metadata: Value = serde_json::from_str(&metadata).map_err(|_| INDEX)?;
        let name = metadata["refName"].as_str().ok_or(INDEX)?;
        repository::insert_ref(
            db,
            &from,
            name,
            "http_route",
            line.unwrap_or(0),
            col.unwrap_or(0),
            None,
            (&path, &language),
        )?;
    }
    sql(db.execute(
        "DELETE FROM edges WHERE CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.refKind') END='http_route'", [],
    ))?;
    Ok(())
}

pub(super) fn pending(db: &Connection, context: &OperationContext) -> Result<Vec<i64>, Failure> {
    let mut statement = sql(db.prepare(
        "SELECT id FROM unresolved_refs WHERE reference_kind='http_route' AND status='pending' ORDER BY id LIMIT 20001",
    ))?;
    let mut ids = Vec::new();
    for row in sql(statement.query_map([], |row| row.get(0)))? {
        context.check()?;
        if ids.len() >= MAX_ROWS {
            return Err(LIMIT);
        }
        ids.push(sql(row)?);
    }
    Ok(ids)
}

#[cfg(test)]
#[path = "http_routes_tests.rs"]
mod tests;
