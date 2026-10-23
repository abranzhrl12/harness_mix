use super::super::project::{Failure, Project};
use super::graph_kinds;
use super::model::Candidate;
use harness_domain::execution::OperationContext;
use rusqlite::{Connection, OpenFlags, limits::Limit};
use std::collections::BTreeMap;
use std::time::Duration;

const MAX_CANDIDATES: usize = 200;
const MAX_INDEX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SCHEMA_VERSION: i64 = 11;

fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// Discover candidate symbols in the graph for one canonical kind.
///
/// `Ok(None)` means "no usable index for this request": the caller falls back
/// to the live path, which stays authoritative.
pub(in crate::adapters) fn discover(
    project: &Project,
    canonical: &str,
    name: Option<&str>,
    scope: &str,
    limit: usize,
    context: &OperationContext,
) -> Result<Option<Vec<Candidate>>, Failure> {
    let kinds = graph_kinds(canonical);
    if kinds.is_empty() {
        return Ok(None);
    }
    context.check()?;
    let path = match project.target_optional(".codegraph/codegraph.db") {
        Ok(path) => path,
        Err(_) => return Ok(None),
    };
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_INDEX_BYTES => {}
        _ => return Ok(None),
    }
    #[cfg(windows)]
    let _database_pin = match super::super::index_lock::open_database_pin(&path, context) {
        Ok(pin) => pin,
        Err(_) => {
            context.check()?;
            return Ok(None);
        }
    };
    let db = match Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    ) {
        Ok(db) => db,
        Err(_) => return Ok(None),
    };
    let _ = db.busy_timeout(Duration::from_millis(super::super::INDEX_BUSY_TIMEOUT_MS));
    let _ = db.set_limit(Limit::SQLITE_LIMIT_LENGTH, 65536);
    let _ = db.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 8192);
    let _ = db.execute_batch(
        "PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-4096;",
    );
    if !has_tables(&db)? || !supported_version(&db)? {
        return Ok(None);
    }
    let columns = node_columns(&db)?;
    if !columns.iter().any(|name| name == "name") || !columns.iter().any(|name| name == "file_path")
    {
        return Ok(None);
    }
    let scope_filter = normalize_scope(scope);
    if !scope_covers(&db, &scope_filter)? {
        // The index does not cover this scope; the live path stays authoritative.
        return Ok(None);
    }
    let kinds =
        serde_json::to_string(kinds).map_err(|_| ("INTERNAL", "kind list serialization"))?;
    let name = name.map(str::to_owned);
    let selected = if scope_filter == "." {
        let sql = "SELECT id,name,kind,file_path,start_line,{qualified},{end_line},{signature},{docstring},{language} \
             FROM nodes WHERE kind IN (SELECT value FROM json_each(?1)) \
             AND (?2 IS NULL OR name = ?2 COLLATE BINARY) \
             ORDER BY file_path,start_line,id LIMIT ?3";
        query_candidates(&db, sql, kinds, name, limit, None, None)?
    } else {
        let sql = "SELECT id,name,kind,file_path,start_line,{qualified},{end_line},{signature},{docstring},{language} \
             FROM nodes WHERE kind IN (SELECT value FROM json_each(?1)) \
             AND (?2 IS NULL OR name = ?2 COLLATE BINARY) \
             AND (file_path LIKE ?3 ESCAPE '\\' OR file_path = ?4 COLLATE BINARY) \
             ORDER BY file_path,start_line,id LIMIT ?5";
        query_candidates(
            &db,
            sql,
            kinds,
            name,
            limit,
            Some(format!("{}/%", escape_like(&scope_filter))),
            Some(scope_filter.clone()),
        )?
    };
    let hashes = indexed_hashes(&db, &selected)?;
    let mut candidates = Vec::new();
    for mut candidate in selected {
        candidate.indexed_hash = hashes.get(&candidate.file_path).cloned();
        if !allowed_path(project, &candidate.file_path)? {
            continue;
        }
        candidates.push(candidate);
    }
    Ok(Some(candidates))
}

fn query_candidates(
    db: &Connection,
    sql: &str,
    kinds: String,
    name: Option<String>,
    limit: usize,
    scope_prefix: Option<String>,
    scope_exact: Option<String>,
) -> Result<Vec<Candidate>, Failure> {
    let columns = node_columns(db)?;
    let qualified = optional("qualified_name", &columns);
    let end_line = optional("end_line", &columns);
    let signature = optional("signature", &columns);
    let docstring = optional("docstring", &columns);
    let language = optional("language", &columns);
    let sql = sql
        .replace("{qualified}", &qualified)
        .replace("{end_line}", &end_line)
        .replace("{signature}", &signature)
        .replace("{docstring}", &docstring)
        .replace("{language}", &language);
    let mut statement = db
        .prepare(&sql)
        .map_err(|_| ("INDEX", "symbol discovery query failed"))?;
    let rows = match (scope_prefix, scope_exact) {
        (Some(prefix), Some(exact)) => statement
            .query_map(
                rusqlite::params![
                    kinds,
                    name,
                    prefix,
                    exact,
                    (limit.min(MAX_CANDIDATES)) as i64
                ],
                map_row,
            )
            .map_err(|_| ("INDEX", "symbol discovery query failed"))?
            .flatten()
            .collect::<Vec<Candidate>>(),
        _ => statement
            .query_map(
                rusqlite::params![kinds, name, (limit.min(MAX_CANDIDATES)) as i64],
                map_row,
            )
            .map_err(|_| ("INDEX", "symbol discovery query failed"))?
            .flatten()
            .collect::<Vec<Candidate>>(),
    };
    Ok(rows)
}

fn normalize_scope(scope: &str) -> String {
    let scope = scope.trim().replace('\\', "/");
    if scope.is_empty() || scope == "." {
        ".".to_owned()
    } else {
        scope.trim_end_matches('/').to_owned()
    }
}

fn has_tables(db: &Connection) -> Result<bool, Failure> {
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name IN ('schema_versions','nodes','files') AND type='table' AND upper(sql) LIKE 'CREATE TABLE%'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| ("INDEX", "index schema is unreadable"))?;
    Ok(count == 3)
}

fn supported_version(db: &Connection) -> Result<bool, Failure> {
    let version: Option<i64> = db
        .query_row("SELECT max(version) FROM schema_versions", [], |row| {
            row.get(0)
        })
        .ok();
    Ok(matches!(version, Some(version) if (1..=MAX_SCHEMA_VERSION).contains(&version)))
}

fn node_columns(db: &Connection) -> Result<Vec<String>, Failure> {
    let mut statement = db
        .prepare("SELECT name FROM pragma_table_info('nodes')")
        .map_err(|_| ("INDEX", "index schema is unreadable"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| ("INDEX", "index schema is unreadable"))?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|_| ("INDEX", "index schema is unreadable"))?;
    Ok(columns)
}

/// Column expression for an optional node attribute.
fn optional(column: &str, columns: &[String]) -> String {
    if columns.iter().any(|name| name == column) {
        column.to_owned()
    } else {
        format!("NULL AS {column}")
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Candidate> {
    let text = |index: usize| row.get::<_, Option<String>>(index).ok().flatten();
    let number = |index: usize| row.get::<_, Option<i64>>(index).ok().flatten().unwrap_or(0);
    Ok(Candidate {
        id: text(0).unwrap_or_default(),
        name: text(1).unwrap_or_default(),
        kind: text(2).unwrap_or_default(),
        file_path: text(3).unwrap_or_default(),
        start_line: number(4).max(0) as u32,
        qualified_name: text(5).unwrap_or_default(),
        end_line: number(6).max(0) as u32,
        signature: text(7).unwrap_or_default(),
        docstring: text(8).unwrap_or_default(),
        language: text(9).unwrap_or_default(),
        indexed_hash: None,
    })
}

/// Does the index contain any file inside the requested scope?
fn scope_covers(db: &Connection, scope: &str) -> Result<bool, Failure> {
    let count: i64 = if scope == "." {
        db.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .unwrap_or(0)
    } else {
        let prefix = format!("{}/%", escape_like(scope));
        db.query_row(
            "SELECT count(*) FROM files WHERE path LIKE ?1 ESCAPE '\\' OR path = ?2 COLLATE BINARY",
            rusqlite::params![prefix, scope],
            |row| row.get(0),
        )
        .unwrap_or(0)
    };
    Ok(count > 0)
}

/// Indexed content hashes for the candidate files.
fn indexed_hashes(
    db: &Connection,
    candidates: &[Candidate],
) -> Result<BTreeMap<String, String>, Failure> {
    let mut hashes = BTreeMap::new();
    if candidates.is_empty() {
        return Ok(hashes);
    }
    let paths: Vec<String> = candidates
        .iter()
        .map(|candidate| candidate.file_path.clone())
        .collect();
    let json =
        serde_json::to_string(&paths).map_err(|_| ("INTERNAL", "path list serialization"))?;
    let mut statement = db
        .prepare(
            "SELECT path,content_hash FROM files WHERE path IN (SELECT value FROM json_each(?1))",
        )
        .map_err(|_| ("INDEX", "index schema is unreadable"))?;
    let rows = statement
        .query_map(rusqlite::params![json], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            ))
        })
        .map_err(|_| ("INDEX", "index schema is unreadable"))?;
    for row in rows.flatten() {
        if !row.1.is_empty() {
            hashes.insert(row.0, row.1);
        }
    }
    Ok(hashes)
}

/// Reject indexed paths that the workspace policy must never surface.
fn allowed_path(project: &Project, relative: &str) -> Result<bool, Failure> {
    if relative.is_empty()
        || relative.len() > 1000
        || relative.chars().any(char::is_control)
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Ok(false);
    }
    if super::super::project::external_restricted(relative)
        || super::super::project::external_sensitive(relative)
    {
        return Ok(false);
    }
    let lowered = relative.to_lowercase();
    for segment in lowered.split('/') {
        if matches!(segment, ".git" | ".hg" | ".svn")
            || segment.starts_with(".env")
            || segment.starts_with("credential")
            || segment.starts_with("secret")
            || segment == "id_rsa"
            || segment == "id_ed25519"
            || segment == "oauth_creds.json"
            || segment == "mcp_config.json"
        {
            return Ok(false);
        }
    }
    Ok(project.target_optional(relative).is_ok())
}
