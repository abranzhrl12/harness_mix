//! Atomic, explicit-file CodeGraph publication. No migrations or source writes.
use super::{index_extract, index_inventory, index_sync, project::Project};
use harness_domain::{Failure, execution::OperationContext, hash};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const INDEX: Failure = (
    "SYNC_INDEX",
    "CodeGraph index incompatible, busy or invalid; no raw database diagnostics returned",
);
const LIMIT: Failure = (
    "SYNC_LIMIT",
    "Native publication exceeded its bounded graph budget",
);
const UNCERTAIN: Failure = (
    "SYNC_COMMIT_UNCERTAIN",
    "Index commit or rollback could not be confirmed; inspect the index before retrying",
);
const MAX_ROWS: usize = 20_000;

#[path = "index_imports.rs"]
mod imports;

/// Apply a bounded native subset of the current plan. Transactions are per
/// file, NOT per workspace. Once any file commits, subsequent errors/cancellation
/// are returned in the JSON report alongside committed paths, never as a bare
/// error that could imply the earlier files were rolled back.
pub fn sync_workspace(root: &Path, context: &OperationContext) -> Result<Value, Failure> {
    context.check()?;
    let project = Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?;
    let config = project.target_optional("codegraph.json")?;
    match std::fs::symlink_metadata(config) {
        Ok(_) => {
            return Err((
                "SYNC_SCOPE",
                "Automatic native sync does not yet implement codegraph.json; use an explicitly reviewed --sync-file scope",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err((
                "SYNC_SCOPE",
                "Cannot establish CodeGraph configuration scope",
            ));
        }
    }
    let planned = index_sync::plan_workspace(root, context)?;
    let unchecked = planned["plan"]["unchecked"].as_array().ok_or(INDEX)?.len();
    let mut paths = BTreeSet::new();
    let mut unsupported = 0;
    for key in ["added", "modified", "retryExtraction", "removed"] {
        for path in planned["plan"][key].as_array().ok_or(INDEX)? {
            let path = path.as_str().ok_or(INDEX)?;
            if index_extract::language(path).is_some() {
                paths.insert(path.to_owned());
            } else {
                unsupported += 1;
            }
        }
    }
    let total = paths.len();
    let mut results = Vec::new();
    let mut failure = None;
    for path in paths.iter().take(32) {
        let applied = context
            .check()
            .and_then(|()| sync_file(root, path, context));
        match applied {
            Ok(result) => results.push(result),
            Err((code, _)) => {
                failure = Some(
                    json!({"path":path,"code":code,"commitState":if code==UNCERTAIN.0 {"unknown"}else{"not_committed"}}),
                );
                break;
            }
        }
    }
    let changed = results
        .iter()
        .filter(|result| result["changed"] == true)
        .count();
    let native_complete = failure.is_none() && results.len() == total;
    Ok(
        json!({"mode":"sync_native_subset","files":results,"failure":failure,
        "changedFilesCommitted":changed,"nativeCandidates":total,"nativeFilesRemaining":total-results.len(),
        "maxFiles":32,"selectedScopeApplied":native_complete,"workspaceTransactionAtomic":false,
        "unsupportedChangedFiles":unsupported,"uncheckedFiles":unchecked,
        "restrictedIndexedFiles":planned["inventory"]["restrictedIndexedFiles"],
        "discoveryComplete":planned["inventory"]["discoveryComplete"],
        "synchronized":false,"globalFreshnessCertified":false,"fullProviderScope":false,
        "resolutionPolicy":"native_exact_name_and_relative_named_import_subset"}),
    )
}

fn sql<T>(value: rusqlite::Result<T>) -> Result<T, Failure> {
    value.map_err(|_| INDEX)
}
fn string(value: &Value, key: &str) -> Result<String, Failure> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 65536)
        .map(str::to_owned)
        .ok_or(INDEX)
}
fn opt_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, Failure> {
    value[key].as_array().ok_or(INDEX)
}

/// The writer deliberately supports only Windows, where ancestors and the
/// final DB entry can be pinned against replacement. The extraction core is portable.
pub fn sync_file(
    root: &Path,
    relative: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    #[cfg(not(windows))]
    {
        let _ = (root, relative, context);
        Err((
            "UNSUPPORTED",
            "Native index publication requires Windows path pinning",
        ))
    }
    #[cfg(windows)]
    {
        sync_file_windows(root, relative, context)
    }
}

#[cfg(windows)]
fn sync_file_windows(
    root: &Path,
    relative: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    sync_file_with(root, relative, context, || {})
}

#[cfg(windows)]
fn sync_file_with(
    root: &Path,
    relative: &str,
    context: &OperationContext,
    before_recheck: impl FnOnce(),
) -> Result<Value, Failure> {
    use std::{
        fs::File,
        os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
    };
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
    };
    context.check()?;
    if relative.is_empty()
        || relative.len() > 1000
        || relative.contains('\\')
        || relative.chars().any(char::is_control)
        || relative.split('/').any(|s| matches!(s, "" | "." | ".."))
        || index_inventory::restricted(relative)
    {
        return Err(("PATH", "Invalid or restricted synchronization source path"));
    }
    let language = index_extract::language(relative).ok_or((
        "SYNC_LANGUAGE",
        "No native extractor for this source language",
    ))?;
    let project = Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?;
    let source = read_source(&project, relative, context)?;
    let extracted = source
        .as_ref()
        .map(|bytes| {
            let text =
                std::str::from_utf8(bytes).map_err(|_| ("ENCODING", "Source is not UTF-8"))?;
            index_extract::extract(relative, text, context)
        })
        .transpose()?;
    let path = project
        .target(".codegraph/codegraph.db")
        .map_err(|_| INDEX)?;
    for suffix in ["-wal", "-shm", "-journal"] {
        project
            .target_optional(&format!(".codegraph/codegraph.db{suffix}"))
            .map_err(|_| INDEX)?;
    }
    let _pins = super::edit_commit::pin_directory(&project, path.parent().ok_or(INDEX)?)?;
    // Deny DELETE sharing so SQLite's path cannot be replaced after validation.
    let pin = File::options()
        .read(true)
        .share_mode(3)
        .custom_flags(0x0020_0000)
        .open(&path)
        .map_err(|_| INDEX)?;
    let metadata = pin.metadata().map_err(|_| INDEX)?;
    if !metadata.is_file()
        || metadata.len() > 512 * 1024 * 1024
        || metadata.file_attributes() & 0x400 != 0
    {
        return Err(INDEX);
    }
    let mut identity = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(HANDLE(pin.as_raw_handle()), &mut identity)
            .map_err(|_| INDEX)?;
    }
    if identity.nNumberOfLinks != 1 {
        return Err(("PATH", "Hard-linked indexes cannot be updated"));
    }
    // Pin deterministic sidecar names too: an attacker must not substitute a
    // junction/symlink between validation and SQLite opening a writable WAL.
    // WAL mode doesn't need to unlink these files at commit. They may remain
    // after closing; this is a valid SQLite WAL layout, not source modification.
    let mut sidecar_pins = Vec::new();
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = project.target_optional(&format!(".codegraph/codegraph.db{suffix}"))?;
        let options = || {
            let mut options = File::options();
            options
                .read(true)
                .write(true)
                .share_mode(3)
                .custom_flags(0x0020_0000);
            options
        };
        let guard = match options().create_new(true).open(&sidecar) {
            Ok(guard) => guard,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                options().open(&sidecar).map_err(|_| INDEX)?
            }
            Err(_) => return Err(INDEX),
        };
        let meta = guard.metadata().map_err(|_| INDEX)?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(guard.as_raw_handle()), &mut info)
                .map_err(|_| INDEX)?;
        }
        if !meta.is_file() || meta.file_attributes() & 0x400 != 0 || info.nNumberOfLinks != 1 {
            return Err(("PATH", "Linked SQLite sidecars cannot be updated"));
        }
        sidecar_pins.push(guard);
    }
    let mut db = sql(Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    ))?;
    sql(db.busy_timeout(Duration::from_millis(100)))?;
    let journal: String = sql(db.query_row("PRAGMA journal_mode", [], |r| r.get(0)))?;
    if journal != "wal" {
        return Err((
            "SYNC_JOURNAL",
            "Native publication requires an existing WAL-mode CodeGraph index",
        ));
    }
    sql(db.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH, 1024 * 1024))?;
    sql(db.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH, 16384))?;
    let cancel = context.clone();
    sql(db.progress_handler(1000, Some(move || cancel.check().is_err())))?;
    sql(db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA synchronous=FULL; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-4096;"))?;
    let tx = sql(db.transaction_with_behavior(TransactionBehavior::Immediate))?;
    let run = || {
        validate_schema(&tx)?;
        // Only the three exact provider FTS triggers are accepted. SQLite FTS5
        // triggers require trusted_schema; it is enabled only after validation.
        sql(tx.execute_batch("PRAGMA trusted_schema=ON"))?;
        context.check()?;
        let (mut result, import_sources) = publish(
            &tx,
            &project,
            relative,
            language,
            source.as_deref(),
            extracted.as_ref(),
            context,
        )?;
        before_recheck();
        import_sources.recheck(&project, context)?;
        let observed = read_source(&project, relative, context)?;
        if observed.as_ref().map(|b| hash(b)) != source.as_ref().map(|b| hash(b)) {
            return Err((
                "STALE",
                "Source changed before index publication; transaction rolled back",
            ));
        }
        context.check()?;
        let violations: bool = sql(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check LIMIT 1)",
            [],
            |row| row.get(0),
        ))?;
        if violations {
            return Err(("SYNC_INTEGRITY", "Index foreign-key integrity check failed"));
        }
        result["sourceSnapshotAtomic"] = json!(false);
        Ok(result)
    };
    let prepared = run();
    match prepared {
        Err(failure) => {
            // Cancellation must not prevent the rollback itself.
            tx.progress_handler(0, None::<fn() -> bool>)
                .map_err(|_| UNCERTAIN)?;
            tx.rollback().map_err(|_| UNCERTAIN)?;
            drop(sidecar_pins);
            Err(context.check().err().unwrap_or(failure))
        }
        Ok(mut result) => {
            // Commit point: don't report a post-commit cancellation as no write.
            tx.progress_handler(0, None::<fn() -> bool>)
                .map_err(|_| UNCERTAIN)?;
            tx.commit().map_err(|_| UNCERTAIN)?;
            drop(sidecar_pins);
            result["committed"] = json!(true);
            Ok(result)
        }
    }
}

fn read_source(
    project: &Project,
    path: &str,
    context: &OperationContext,
) -> Result<Option<Vec<u8>>, Failure> {
    context.check()?;
    let target = project.target_optional(path)?;
    match std::fs::symlink_metadata(&target) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(("IO", "Source metadata unavailable; absence not established")),
        Ok(metadata) if !metadata.is_file() => {
            return Err(("PATH", "Source is not a regular file"));
        }
        Ok(_) => {}
    }
    #[cfg(windows)]
    let _pins = super::edit_commit::pin_directory(project, target.parent().ok_or(INDEX)?)?;
    project
        .bytes_with_limit(path, context, index_sync::MAX_SOURCE_BYTES as u64)
        .map(Some)
}

fn normalized(sql: &str) -> String {
    sql.to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != ';')
        .collect::<String>()
        .replace("ifnotexists", "")
}
fn validate_schema(db: &Connection) -> Result<(), Failure> {
    let ordinary: i64 = sql(db.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('schema_versions','nodes','edges','files','unresolved_refs','project_metadata') AND type='table' AND upper(sql) LIKE 'CREATE TABLE%'", [], |r| r.get(0)))?;
    if ordinary != 6 {
        return Err(INDEX);
    }
    let version: i64 =
        sql(db.query_row("SELECT max(version) FROM schema_versions", [], |r| r.get(0)))?;
    if version != 11 {
        return Err(INDEX);
    }
    let mut statement =
        sql(db.prepare("SELECT name,sql FROM sqlite_schema WHERE type='trigger' LIMIT 4"))?;
    let rows = sql(statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }))?;
    let mut trigger_count = 0;
    for row in rows {
        let (name, body) = sql(row)?;
        let expected = match name.as_str() {
            "nodes_ai" => {
                "CREATE TRIGGER nodes_ai AFTER INSERT ON nodes BEGIN INSERT INTO nodes_fts(rowid,id,name,qualified_name,docstring,signature) VALUES(NEW.rowid,NEW.id,NEW.name,NEW.qualified_name,NEW.docstring,NEW.signature); END"
            }
            "nodes_ad" => {
                "CREATE TRIGGER nodes_ad AFTER DELETE ON nodes BEGIN INSERT INTO nodes_fts(nodes_fts,rowid,id,name,qualified_name,docstring,signature) VALUES('delete',OLD.rowid,OLD.id,OLD.name,OLD.qualified_name,OLD.docstring,OLD.signature); END"
            }
            "nodes_au" => {
                "CREATE TRIGGER nodes_au AFTER UPDATE ON nodes BEGIN INSERT INTO nodes_fts(nodes_fts,rowid,id,name,qualified_name,docstring,signature) VALUES('delete',OLD.rowid,OLD.id,OLD.name,OLD.qualified_name,OLD.docstring,OLD.signature); INSERT INTO nodes_fts(rowid,id,name,qualified_name,docstring,signature) VALUES(NEW.rowid,NEW.id,NEW.name,NEW.qualified_name,NEW.docstring,NEW.signature); END"
            }
            _ => {
                return Err((
                    "SYNC_SCHEMA",
                    "Unrecognized index trigger; publication refused",
                ));
            }
        };
        if normalized(&body) != normalized(expected) {
            return Err(("SYNC_SCHEMA", "Modified index trigger; publication refused"));
        }
        trigger_count += 1;
    }
    if trigger_count != 3 {
        return Err(("SYNC_SCHEMA", "Provider full-text triggers are missing"));
    }
    // Refuse virtual/view substitutions of writable tables via the ordinary-table
    // check above. If FTS exists, require the provider's exact external-content ABI.
    let fts: Option<String> = sql(db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='nodes_fts'",
            [],
            |r| r.get(0),
        )
        .optional())?;
    if let Some(fts) = fts {
        let expected = "CREATE VIRTUAL TABLE nodes_fts USING fts5(id,name,qualified_name,docstring,signature,content='nodes',content_rowid='rowid')";
        if normalized(&fts) != normalized(expected) {
            return Err(("SYNC_SCHEMA", "Unsupported full-text index definition"));
        }
    } else {
        return Err(("SYNC_SCHEMA", "Provider full-text index is missing"));
    }
    Ok(())
}

#[derive(Clone)]
struct Identity {
    id: String,
    key: String,
}
fn identities(db: &Connection, path: &str) -> Result<Vec<Identity>, Failure> {
    let mut statement = sql(db.prepare("SELECT id,kind,qualified_name,ifnull(signature,'') FROM nodes WHERE file_path=?1 LIMIT 20001"))?;
    let rows = sql(statement.query_map([path], |r| {
        Ok(Identity {
            id: r.get(0)?,
            key: format!(
                "{}\0{}\0{}",
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?
            ),
        })
    }))?;
    let mut result = Vec::new();
    let mut reserved = 0usize;
    for row in rows {
        let row = sql(row)?;
        reserved += row.id.len() + row.key.len();
        if result.len() >= MAX_ROWS || reserved > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        result.push(row);
    }
    Ok(result)
}
fn identity_map(
    prior: Vec<Identity>,
    extracted: Option<&Value>,
) -> Result<BTreeMap<String, String>, Failure> {
    let mut before: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut after: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for node in prior {
        before.entry(node.key).or_default().push(node.id);
    }
    if let Some(value) = extracted {
        for node in array(value, "nodes")? {
            let key = format!(
                "{}\0{}\0{}",
                string(node, "kind")?,
                string(node, "qualifiedName")?,
                opt_string(node, "signature").unwrap_or("")
            );
            after.entry(key).or_default().push(string(node, "id")?);
        }
    }
    let mut result = BTreeMap::new();
    for (key, ids) in before {
        if ids.len() == 1
            && let Some(next) = after.get(&key)
            && next.len() == 1
        {
            result.insert(ids[0].clone(), next[0].clone());
        }
    }
    Ok(result)
}

#[derive(Clone)]
struct Incoming {
    source: String,
    target: String,
    kind: String,
    line: Option<i64>,
    col: Option<i64>,
    metadata: Option<String>,
    provenance: Option<String>,
    path: String,
    language: String,
}
fn incoming(db: &Connection, path: &str) -> Result<Vec<Incoming>, Failure> {
    let mut statement = sql(db.prepare("SELECT e.source,e.target,e.kind,e.line,e.col,e.metadata,e.provenance,s.file_path,s.language FROM edges e JOIN nodes t ON t.id=e.target JOIN nodes s ON s.id=e.source WHERE t.file_path=?1 AND s.file_path!=?1 AND e.kind!='contains' LIMIT 20001"))?;
    let rows = sql(statement.query_map([path], |r| {
        Ok(Incoming {
            source: r.get(0)?,
            target: r.get(1)?,
            kind: r.get(2)?,
            line: r.get(3)?,
            col: r.get(4)?,
            metadata: r.get(5)?,
            provenance: r.get(6)?,
            path: r.get(7)?,
            language: r.get(8)?,
        })
    }))?;
    let mut result = Vec::new();
    let mut reserved = 0usize;
    for row in rows {
        let row = sql(row)?;
        reserved += row.source.len()
            + row.target.len()
            + row.kind.len()
            + row.path.len()
            + row.language.len()
            + row.metadata.as_ref().map_or(0, String::len)
            + row.provenance.as_ref().map_or(0, String::len);
        if result.len() >= MAX_ROWS || reserved > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        result.push(row);
    }
    Ok(result)
}
#[allow(clippy::too_many_arguments)] // Mirrors the provider's unresolved_refs row.
fn insert_ref(
    db: &Connection,
    source: &str,
    name: &str,
    kind: &str,
    line: i64,
    col: i64,
    candidates: Option<String>,
    origin: (&str, &str),
) -> Result<i64, Failure> {
    sql(db.execute("INSERT INTO unresolved_refs(from_node_id,reference_name,reference_kind,line,col,candidates,file_path,language,status,name_tail) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending','')",params![source,name,kind,line,col,candidates,origin.0,origin.1]))?;
    Ok(db.last_insert_rowid())
}

fn publish(
    db: &Transaction<'_>,
    project: &Project,
    path: &str,
    language: &str,
    bytes: Option<&[u8]>,
    extracted: Option<&Value>,
    context: &OperationContext,
) -> Result<(Value, imports::Sources), Failure> {
    let record: Option<(String, Option<String>)> = sql(db
        .query_row(
            "SELECT content_hash,errors FROM files WHERE path=?1",
            [path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional())?;
    let clean = record.as_ref().is_some_and(|(_, errors)| {
        errors.as_ref().is_none_or(|e| {
            serde_json::from_str::<Value>(e).is_ok_and(|v| v.as_array().is_some_and(Vec::is_empty))
        })
    });
    if (bytes.is_none() && record.is_none())
        || (clean
            && bytes.is_some_and(|bytes| {
                record
                    .as_ref()
                    .is_some_and(|(digest, _)| *digest == hash(bytes))
            }))
    {
        return Ok((
            json!({"mode":"sync_file","path":path,"changed":false,"committed":false,"synchronized":false,"globalFreshnessCertified":false}),
            imports::Sources::default(),
        ));
    }
    let synthesized:bool = sql(db.query_row("SELECT EXISTS(SELECT 1 FROM edges WHERE (source IN(SELECT id FROM nodes WHERE file_path=?1) OR target IN(SELECT id FROM nodes WHERE file_path=?1)) AND CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.synthesizedBy') IS NOT NULL ELSE 0 END)",[path],|r|r.get(0)))?;
    let third_file:bool = sql(db.query_row("SELECT EXISTS(SELECT 1 FROM edges WHERE CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.synthesizedBy') END IS NOT NULL AND CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.registeredAt') END >= ?1 AND CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.registeredAt') END < ?2)",params![format!("{path}:"),format!("{path};")],|r|r.get(0)))?;
    let synthesis_table:bool = sql(db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='synthesis_inputs' AND upper(sql) LIKE 'CREATE TABLE%')",[],|r|r.get(0)))?;
    let prior_synthesis_input = if synthesis_table {
        sql(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM synthesis_inputs WHERE file_path=?1)",
            [path],
            |r| r.get::<_, bool>(0),
        ))?
    } else {
        false
    };
    let framework_nodes: bool = sql(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM nodes WHERE file_path=?1 AND kind IN('route','component'))",
        [path],
        |r| r.get(0),
    ))?;
    if synthesized || third_file || prior_synthesis_input || framework_nodes {
        return Err((
            "SYNC_UNSUPPORTED_RELATION",
            "Affected framework/synthesized graph data requires the provider resolver; index unchanged",
        ));
    }
    let prior = identities(db, path)?;
    let paired = identity_map(prior, extracted)?;
    let incoming = incoming(db, path)?;
    // Capture original refs BEFORE deleting nodes/edges; never turn receiver.foo
    // into foo. Legacy unpaired edges without stamps block the entire transaction.
    let mut resurrect = Vec::new();
    let mut retained = Vec::new();
    for mut edge in incoming {
        context.check()?;
        if let Some(target) = paired.get(&edge.target) {
            edge.target = target.clone();
            retained.push(edge);
        } else {
            let metadata: Value = edge
                .metadata
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok())
                .ok_or((
                    "SYNC_LEGACY_REFERENCE",
                    "Cannot recover original reference for an incoming edge; index unchanged",
                ))?;
            let name = string(&metadata, "refName").map_err(|_| {
                (
                    "SYNC_LEGACY_REFERENCE",
                    "Incoming edge lacks original reference metadata; index unchanged",
                )
            })?;
            let kind = opt_string(&metadata, "refKind")
                .unwrap_or(&edge.kind)
                .to_owned();
            resurrect.push((edge, name, kind));
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| INDEX)?
        .as_millis() as i64;
    let generated: i64 = sql(db
        .query_row("SELECT generated FROM files WHERE path=?1", [path], |r| {
            r.get(0)
        })
        .optional())?
    .unwrap_or(0);
    // Explicit deletion also supports indexes produced with foreign_keys off.
    sql(db.execute("DELETE FROM unresolved_refs WHERE from_node_id IN(SELECT id FROM nodes WHERE file_path=?1)",[path]))?;
    sql(db.execute("DELETE FROM edges WHERE source IN(SELECT id FROM nodes WHERE file_path=?1) OR target IN(SELECT id FROM nodes WHERE file_path=?1)",[path]))?;
    sql(db.execute("DELETE FROM nodes WHERE file_path=?1", [path]))?;
    let mut refs = BTreeSet::new();
    let mut node_count = 0;
    if let Some(extracted) = extracted {
        let nodes = array(extracted, "nodes")?;
        if nodes.len() > MAX_ROWS {
            return Err(LIMIT);
        }
        for node in nodes {
            context.check()?;
            sql(db.execute("INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,docstring,signature,visibility,is_exported,is_async,is_static,is_abstract,decorators,type_parameters,return_type,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)", params![string(node,"id")?,string(node,"kind")?,string(node,"name")?,string(node,"qualifiedName")?,path,language,node["startLine"].as_i64(),node["endLine"].as_i64(),node["startColumn"].as_i64(),node["endColumn"].as_i64(),opt_string(node,"docstring"),opt_string(node,"signature"),opt_string(node,"visibility"),node["isExported"].as_bool().unwrap_or(false),node["isAsync"].as_bool().unwrap_or(false),node["isStatic"].as_bool().unwrap_or(false),node["isAbstract"].as_bool().unwrap_or(false),node.get("decorators").map(Value::to_string),node.get("typeParameters").map(Value::to_string),opt_string(node,"returnType"),now]))?;
        }
        node_count = nodes.len();
        for edge in array(extracted, "edges")? {
            context.check()?;
            sql(db.execute("INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![string(edge,"source")?,string(edge,"target")?,string(edge,"kind")?,edge["line"].as_i64(),edge["column"].as_i64(),edge.get("metadata").map(Value::to_string),opt_string(edge,"provenance")]))?;
        }
        for reference in array(extracted, "unresolvedReferences")? {
            context.check()?;
            refs.insert(insert_ref(
                db,
                &string(reference, "fromNodeId")?,
                &string(reference, "referenceName")?,
                &string(reference, "referenceKind")?,
                reference["line"].as_i64().ok_or(INDEX)?,
                reference["column"].as_i64().ok_or(INDEX)?,
                reference.get("candidates").map(Value::to_string),
                (path, language),
            )?);
        }
        let bytes = bytes.ok_or(INDEX)?;
        // modified_at=0 disables CodeGraph's size/mtime fast-path on the next
        // provider sync; don't invent a file timestamp from extraction time.
        sql(db.execute("INSERT INTO files(path,content_hash,language,size,modified_at,indexed_at,node_count,errors,generated) VALUES(?1,?2,?3,?4,0,?5,?6,'[]',?7) ON CONFLICT(path) DO UPDATE SET content_hash=excluded.content_hash,language=excluded.language,size=excluded.size,modified_at=0,indexed_at=excluded.indexed_at,node_count=excluded.node_count,errors=excluded.errors,generated=excluded.generated",params![path,hash(bytes),language,bytes.len() as i64,now,node_count as i64,generated]))?;
    } else {
        sql(db.execute("DELETE FROM files WHERE path=?1", [path]))?;
    }
    let retained_count = retained.len();
    let resurrected_count = resurrect.len();
    for edge in retained {
        context.check()?;
        sql(db.execute("INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![edge.source,edge.target,edge.kind,edge.line,edge.col,edge.metadata,edge.provenance]))?;
    }
    for (edge, name, kind) in resurrect {
        context.check()?;
        refs.insert(insert_ref(
            db,
            &edge.source,
            &name,
            &kind,
            edge.line.unwrap_or(0),
            edge.col.unwrap_or(0),
            None,
            (&edge.path, &edge.language),
        )?);
        if refs.len() > MAX_ROWS {
            return Err(LIMIT);
        }
    }
    let import_rechecks = if matches!(language, "typescript" | "tsx" | "javascript" | "jsx") {
        imports::invalidate(db)?
    } else {
        Vec::new()
    };
    let import_recheck_count = import_rechecks.len();
    refs.extend(import_rechecks);
    if refs.len() > MAX_ROWS {
        return Err(LIMIT);
    }
    // Retry pending/failed refs whose exact name became available in this file.
    let mut statement=sql(db.prepare("SELECT id FROM unresolved_refs WHERE reference_name IN(SELECT name FROM nodes WHERE file_path=?1) OR reference_name IN(SELECT qualified_name FROM nodes WHERE file_path=?1) OR (language IN ('typescript','tsx','javascript','jsx') AND file_path IN(SELECT file_path FROM nodes WHERE kind='import')) LIMIT 20001"))?;
    for row in sql(statement.query_map([path], |r| r.get::<_, i64>(0)))? {
        refs.insert(sql(row)?);
        if refs.len() > MAX_ROWS {
            return Err(LIMIT);
        }
    }
    let mut resolved = 0;
    let mut import_sources = imports::Sources::default();
    for id in &refs {
        context.check()?;
        resolved += usize::from(resolve_exact(
            db,
            *id,
            project,
            context,
            &mut import_sources,
        )?);
    }
    // Invalidate the global provider stamp. This single-file subset is not a
    // full language/framework resolver or a whole-workspace synchronization.
    sql(db.execute(
        "DELETE FROM project_metadata WHERE key IN('indexed_at_commit','indexed_dirty_paths')",
        [],
    ))?;
    sql(db.execute("INSERT INTO project_metadata(key,value,updated_at) VALUES('native_sync_state',?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",params![json!({"scope":"explicit_file","resolution":"exact_name_and_relative_named_import_subset","globalFreshnessCertified":false}).to_string(),now]))?;
    Ok((
        json!({"mode":"sync_file","path":path,"changed":true,"deleted":extracted.is_none(),"nodesWritten":node_count,"incomingEdgesRetained":retained_count,"incomingReferencesRecovered":resurrected_count,"importReferencesRechecked":import_recheck_count,"referencesAttempted":refs.len(),"referencesResolved":resolved,"referencesPending":refs.len()-resolved,"resolutionPolicy":"native_exact_name_and_relative_named_import_subset","committed":false,"synchronized":false,"globalFreshnessCertified":false}),
        import_sources,
    ))
}

type ResolutionReference = (String, String, String, i64, i64, String, String, String);
fn resolve_exact(
    db: &Connection,
    id: i64,
    project: &Project,
    context: &OperationContext,
    sources: &mut imports::Sources,
) -> Result<bool, Failure> {
    let reference:Option<ResolutionReference> = sql(db.query_row("SELECT r.from_node_id,r.reference_name,r.reference_kind,r.line,r.col,r.file_path,r.language,n.qualified_name FROM unresolved_refs r JOIN nodes n ON n.id=r.from_node_id WHERE r.id=?1",[id],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional())?;
    let Some((from, name, kind, line, col, path, language, owner)) = reference else {
        return Ok(false);
    };
    let allowed: &[&str] = match kind.as_str() {
        "calls" | "function_ref" => &["function", "method"],
        "instantiates" => &["class", "struct"],
        "extends" | "implements" => &["class", "struct", "interface", "trait"],
        _ => return Ok(false),
    };
    let js = matches!(
        language.as_str(),
        "typescript" | "tsx" | "javascript" | "jsx"
    );
    let scope = if js {
        imports::scope(db, &path, &name)?
    } else {
        imports::Scope::None
    };
    if matches!(scope, imports::Scope::Pending) {
        return Ok(false);
    }
    let (lookup, imported_path) = match &scope {
        imports::Scope::Binding { path, name } => (name.as_str(), Some(path.as_str())),
        _ => (name.as_str(), None),
    };
    if imported_path.is_some()
        && (kind != "calls" || !sources.permits(db, project, context, &path, &language, &name)?)
    {
        return Ok(false);
    }
    // Receivers/chains are never stripped. Named imports override global names;
    // unsupported import forms defer instead of falling back to a homonym.
    let mut statement=sql(db.prepare("SELECT id,kind,file_path,qualified_name,visibility,is_exported FROM nodes WHERE (name=?1 COLLATE BINARY OR qualified_name=?1 COLLATE BINARY) AND language=?2 ORDER BY file_path,start_line,id LIMIT 65"))?;
    let rows = sql(statement.query_map(params![lookup, language], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<bool>>(5)?.unwrap_or(false),
        ))
    }))?;
    let mut candidates = sql(rows.collect::<rusqlite::Result<Vec<_>>>())?;
    if candidates.len() > 64 {
        return Ok(false);
    }
    candidates.retain(|n| allowed.contains(&n.1.as_str()));
    if let Some(target) = imported_path {
        candidates.retain(|n| n.2 == target && n.5 && n.1 != "method" && n.3 == lookup);
    } else if matches!(scope, imports::Scope::LocalOnly) {
        candidates.retain(|n| n.2 == path);
    }
    // Cross-file private symbols require receiver/import/visibility analysis
    // outside this subset. Keep them pending rather than guessing accessibility.
    candidates.retain(|n| n.2 == path || n.4.as_deref() == Some("public") || n.5);
    let qualified = lookup.contains('.') || lookup.contains("::");
    if qualified {
        candidates.retain(|n| n.3 == lookup);
    } else if imported_path.is_none() {
        let container = owner
            .rsplit_once("::")
            .map(|(p, _)| format!("{p}::"))
            .or_else(|| owner.rsplit_once('.').map(|(p, _)| format!("{p}.")));
        if let Some(container) = container {
            let scoped = candidates
                .iter()
                .filter(|n| n.2 == path && n.3 == format!("{container}{name}"))
                .cloned()
                .collect::<Vec<_>>();
            if !scoped.is_empty() {
                candidates = scoped;
            }
        }
        let local = candidates
            .iter()
            .filter(|n| n.2 == path)
            .cloned()
            .collect::<Vec<_>>();
        if !local.is_empty() {
            candidates = local;
        }
    }
    if candidates.len() != 1 {
        return Ok(false);
    }
    if let Some(target) = imported_path
        && !sources.exported(db, project, context, target, &language, lookup)?
    {
        return Ok(false);
    }
    let edge_kind = if kind == "function_ref" {
        "references"
    } else {
        &kind
    };
    let metadata=json!({"refName":name,"refKind":kind,"resolvedBy":if imported_path.is_some(){"native-relative-named-import-subset"}else{"native-exact-name-subset"},"confidence":0.9}).to_string();
    sql(db.execute("INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,?3,?4,?5,?6,'heuristic')",params![from,candidates[0].0,edge_kind,line,col,metadata]))?;
    sql(db.execute("DELETE FROM unresolved_refs WHERE id=?1", [id]))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) {
        std::fs::create_dir_all(root.join(".codegraph")).unwrap();
        let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
        db.execute_batch(include_str!("../../vendor/codegraph-native/schema.sql"))
            .unwrap();
        db.execute_batch("INSERT INTO schema_versions(version,applied_at) VALUES(11,0); PRAGMA journal_mode=WAL;").unwrap();
        std::fs::write(root.join("a.rs"), "pub fn old() {}\n").unwrap();
        sync_file(root, "a.rs", &OperationContext::default()).unwrap();
    }
    #[test]
    fn source_change_after_sql_writes_rolls_back_index() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        std::fs::write(dir.path().join("a.rs"), "pub fn planned() {}\n").unwrap();
        let result = sync_file_with(dir.path(), "a.rs", &OperationContext::default(), || {
            std::fs::write(dir.path().join("a.rs"), "pub fn concurrent() {}\n").unwrap();
        });
        assert_eq!(result.unwrap_err().0, "STALE");
        let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM nodes WHERE name='old'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM nodes WHERE name='planned'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn cancellation_after_sql_writes_does_not_interrupt_rollback() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        std::fs::write(dir.path().join("a.rs"), "pub fn cancelled() {}\n").unwrap();
        let context = OperationContext::default();
        let result = sync_file_with(dir.path(), "a.rs", &context, || context.cancel());
        assert_eq!(result.unwrap_err().0, "CANCELLED");
        let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM nodes WHERE name='old'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn import_source_changed_after_resolution_rolls_back_target_update() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        std::fs::write(dir.path().join("lib.ts"), "export function run() {}").unwrap();
        sync_file(dir.path(), "lib.ts", &OperationContext::default()).unwrap();
        std::fs::write(
            dir.path().join("app.ts"),
            "import {run as execute} from './lib'; export function caller() { execute(); }",
        )
        .unwrap();
        sync_file(dir.path(), "app.ts", &OperationContext::default()).unwrap();
        let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
        let before: String = db
            .query_row(
                "SELECT content_hash FROM files WHERE path='lib.ts'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        drop(db);
        std::fs::write(dir.path().join("lib.ts"), "\nexport function run() {}").unwrap();
        let result = sync_file_with(dir.path(), "lib.ts", &OperationContext::default(), || {
            std::fs::write(dir.path().join("app.ts"), "import {run as execute} from './lib'; export function caller(execute: () => void) { execute(); }").unwrap();
        });
        assert_eq!(result.unwrap_err().0, "STALE");
        let db = Connection::open(dir.path().join(".codegraph/codegraph.db")).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT content_hash FROM files WHERE path='lib.ts'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            before
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM edges WHERE kind='calls'", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            1
        );
    }
}
