//! Graph-backed symbol discovery certified against the working tree.
//!
//! The index is a cache, never a second source of truth: candidates come from
//! the graph, every candidate file is certified against the working tree, and
//! files that are not fresh are reparsed live before their rows are published.
//!
//! Three phases, in this order:
//!   1. discover: candidate symbols from `nodes` (bounded SQL, no file reads)
//!   2. certify:  compare the indexed content hash with the file on disk
//!   3. repair:   reparse stale files live so published rows stay true
use super::project::{Failure, Project};
use harness_domain::{execution::OperationContext, hash};
use rusqlite::{Connection, OpenFlags, limits::Limit};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

const MAX_CANDIDATES: usize = 200;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_INDEX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SCHEMA_VERSION: i64 = 11;

/// A symbol candidate read from the graph, with the stamp that certifies it.
#[derive(Clone, Debug)]
pub(super) struct Candidate {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub qualified_name: String,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub signature: String,
    pub docstring: String,
    pub language: String,
    pub indexed_hash: Option<String>,
}

/// Freshness of one indexed file relative to the working tree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Verdict {
    /// The indexed content hash still matches the file on disk.
    Fresh,
    /// The file changed after it was indexed.
    Stale,
    /// The indexed file no longer exists.
    Missing,
    /// Freshness could not be established.
    Unknown,
}

impl Verdict {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Verdict::Fresh => "fresh",
            Verdict::Stale => "stale",
            Verdict::Missing => "missing",
            Verdict::Unknown => "unknown",
        }
    }
}

/// Canonical declaration kinds shared by the live parsers and the graph.
///
/// The graph and the language-specific parsers name the same constructs
/// differently (`function` vs `function_item` vs `function_declaration`), so
/// both sides are normalized to one canonical vocabulary before matching.
pub(super) fn canonical_kind(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "function_item"
        | "function"
        | "function_declaration"
        | "function_definition"
        | "method"
        | "method_definition"
        | "method_signature"
        | "constructor" => "function_item",
        "function_signature_item" => "function_signature_item",
        "struct_item" | "struct" | "class" | "class_declaration" | "record" => "struct_item",
        "enum_item" | "enum" | "enum_declaration" => "enum_item",
        "union_item" | "union" => "union_item",
        "trait_item" | "trait" | "interface" | "interface_declaration" => "trait_item",
        "mod_item" | "module" | "namespace" | "internal_module" => "mod_item",
        "type_item" | "type" | "type_alias_declaration" => "type_item",
        "const_item" | "const" | "constant" => "const_item",
        "static_item" | "static" => "static_item",
        "macro_definition" | "macro" => "macro_definition",
        "property_item" | "property" | "property_declaration" => "property_item",
        "variable_item" | "variable" | "field" | "variable_declarator" => "variable_item",
        "impl_item" => "impl_item",
        _ => return None,
    })
}

/// Every kind a selector may ask for: canonical names and their aliases.
pub(super) const SELECTOR_KINDS: &[&str] = &[
    "function_item",
    "function_signature_item",
    "struct_item",
    "enum_item",
    "union_item",
    "trait_item",
    "mod_item",
    "type_item",
    "const_item",
    "static_item",
    "macro_definition",
    "property_item",
    "variable_item",
    "impl_item",
    "function",
    "function_declaration",
    "function_definition",
    "method",
    "method_definition",
    "method_signature",
    "constructor",
    "struct",
    "class",
    "class_declaration",
    "record",
    "enum",
    "enum_declaration",
    "union",
    "trait",
    "interface",
    "interface_declaration",
    "module",
    "namespace",
    "internal_module",
    "type",
    "type_alias_declaration",
    "const",
    "constant",
    "static",
    "macro",
    "property",
    "property_declaration",
    "variable",
    "field",
    "variable_declarator",
];

/// Concrete graph node kinds a canonical kind maps to.
///
/// `impl_item` has no graph representation: Rust impl blocks are not emitted as
/// nodes, so the graph cannot answer that selector and the live path is used.
pub(super) fn graph_kinds(canonical: &str) -> &'static [&'static str] {
    match canonical {
        "function_item" => &["function", "method", "constructor"],
        "function_signature_item" => &["function", "method"],
        "struct_item" => &["struct", "class", "record"],
        "enum_item" => &["enum"],
        "union_item" => &["union"],
        "trait_item" => &["trait", "interface"],
        "mod_item" => &["module", "namespace"],
        "type_item" => &["type"],
        "const_item" => &["constant", "const"],
        "static_item" => &["static"],
        "macro_definition" => &["macro"],
        "property_item" => &["property"],
        "variable_item" => &["variable", "field"],
        _ => &[],
    }
}

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

/// Cached verdict per file, keyed by the observed size and modification time.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified_nanos: u128,
    size: u64,
}

/// Verdicts cached per file: the observed stamp, the indexed hash and the verdict.
type VerdictCache = Mutex<BTreeMap<String, (Stamp, String, Verdict)>>;

fn cache() -> &'static VerdictCache {
    static CACHE: OnceLock<VerdictCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn stamp_of(metadata: &std::fs::Metadata) -> Stamp {
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_nanos());
    Stamp {
        modified_nanos,
        size: metadata.len(),
    }
}

/// Discover candidate symbols in the graph for one canonical kind.
///
/// `Ok(None)` means "no usable index for this request": the caller falls back
/// to the live path, which stays authoritative.
pub(super) fn discover(
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
    let db = match Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    ) {
        Ok(db) => db,
        Err(_) => return Ok(None),
    };
    let _ = db.busy_timeout(Duration::from_millis(100));
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
    if super::project::external_restricted(relative) || super::project::external_sensitive(relative)
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

/// Certify candidate files against the working tree.
///
/// The verdict is cached per file and invalidated by the observed size and
/// modification time, so repeated queries do not re-read unchanged files.
pub(super) fn certify(
    project: &Project,
    candidates: &[Candidate],
    context: &OperationContext,
) -> BTreeMap<String, Verdict> {
    let mut verdicts = BTreeMap::new();
    let mut cache = cache().lock().unwrap_or_else(|error| error.into_inner());
    for candidate in candidates {
        if verdicts.contains_key(&candidate.file_path) {
            continue;
        }
        if let Err((code, _)) = context.check()
            && (code == "CANCELLED" || code == "TIMEOUT")
        {
            verdicts.insert(candidate.file_path.clone(), Verdict::Unknown);
            break;
        }
        let absolute = match project.target_optional(&candidate.file_path) {
            Ok(path) => path,
            Err(_) => {
                verdicts.insert(candidate.file_path.clone(), Verdict::Unknown);
                continue;
            }
        };
        let metadata = match std::fs::metadata(&absolute) {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => {
                verdicts.insert(candidate.file_path.clone(), Verdict::Missing);
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                verdicts.insert(candidate.file_path.clone(), Verdict::Missing);
                continue;
            }
            Err(_) => {
                verdicts.insert(candidate.file_path.clone(), Verdict::Unknown);
                continue;
            }
        };
        if metadata.len() > MAX_FILE_BYTES {
            verdicts.insert(candidate.file_path.clone(), Verdict::Unknown);
            continue;
        }
        let stamp = stamp_of(&metadata);
        let indexed = candidate.indexed_hash.clone().unwrap_or_default();
        if let Some(entry) = cache.get(&candidate.file_path)
            && entry.0 == stamp
            && entry.1 == indexed
        {
            verdicts.insert(candidate.file_path.clone(), entry.2);
            continue;
        }
        let verdict = match project.bytes_with_limit(&candidate.file_path, context, MAX_FILE_BYTES)
        {
            Ok(bytes) => {
                let digest = hash(&bytes);
                match candidate.indexed_hash.as_deref() {
                    Some(expected) if expected == digest => Verdict::Fresh,
                    Some(_) => Verdict::Stale,
                    None => Verdict::Unknown,
                }
            }
            Err(_) => Verdict::Unknown,
        };
        if cache.len() > 4096 {
            cache.clear();
        }
        cache.insert(candidate.file_path.clone(), (stamp, indexed, verdict));
        verdicts.insert(candidate.file_path.clone(), verdict);
    }
    verdicts
}

/// Summary of the certification phase for the response envelope.
pub(super) fn freshness_summary(verdicts: &BTreeMap<String, Verdict>) -> Value {
    let mut summary = json!({"files":0,"fresh":0,"stale":0,"missing":0,"unknown":0});
    for verdict in verdicts.values() {
        summary["files"] = json!(summary["files"].as_u64().unwrap_or(0) + 1);
        summary[verdict.as_str()] = json!(summary[verdict.as_str()].as_u64().unwrap_or(0) + 1);
    }
    summary
}
