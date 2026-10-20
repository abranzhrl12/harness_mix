//! Read existing CodeGraph SQLite indexes without Node, migrations or arbitrary SQL.
use super::{project::Project, symbols};
use harness_domain::{
    Failure,
    contracts::{ExploreArgs, ExploreDirection, ExploreView, ReadSymbolArgs},
    execution::OperationContext,
    results::{prefix, utf16_len},
};
use rusqlite::{Connection, OpenFlags, Row, limits::Limit};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::{Duration, Instant},
};

const INDEX: Failure = (
    "INDEX",
    "CodeGraph index access failed; inspect data.indexDiagnostic for the cause, path and schema",
);
const CONTRACT: Failure = (
    "GRAPH_CONTRACT",
    "CodeGraph returned invalid or restricted graph metadata",
);
const BUDGET: Failure = (
    "GRAPH_LIMIT",
    "CodeGraph query exceeded its bounded read budget",
);

// Pure presentation: no database access, source reads or traversal decisions.
mod presentation {
    pub(super) fn header(symbol: &str) -> String {
        format!(
            "**Exploration: {}**\n\nIndexed relationships; freshness is not certified.\n\n**Source Code**\n",
            symbol.replace(['\r', '\n'], " ")
        )
    }

    pub(super) fn source(path: &str, start: u64, body: &str, truncated: bool) -> String {
        // Pick a fence longer than any backtick run in untrusted source.
        let fence = "`".repeat(
            body.split(|c| c != '`')
                .map(str::len)
                .max()
                .unwrap_or(0)
                .max(2)
                + 1,
        );
        let numbered = body
            .lines()
            .enumerate()
            .map(|(offset, line)| format!("{}\t{line}", start + offset as u64))
            .collect::<Vec<_>>()
            .join("\n");
        let path = path.replace(['\r', '\n', '`'], "_");
        let notice = if truncated {
            "\n> Source fragment truncated; use read_symbol for exact code.\n"
        } else {
            ""
        };
        format!("\n### {path}:{start}\n\n{fence}\n{numbered}\n{fence}\n{notice}")
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn numbers_current_source_without_changing_line_content() {
            let rendered = super::source("a.rs", 7, "fn a() {\r\n    b();\r\n}", false);
            assert!(rendered.contains("7\tfn a() {\n8\t    b();\n9\t}"));
            assert!(!rendered.contains("truncated"));
        }
        #[test]
        fn source_cannot_close_its_fence_and_truncation_is_explicit() {
            let rendered = super::source("a.rs", 1, "```\nhello", true);
            assert!(rendered.contains("\n````\n1\t```"));
            assert!(rendered.contains("Source fragment truncated"));
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Node {
    id: String,
    name: String,
    kind: String,
    file_path: String,
    start_line: i64,
}

// Selection only: deterministic, bounded, and independent of source rendering.
fn context_nodes<'a>(
    targets: &[Node],
    nodes: &'a BTreeMap<String, Node>,
    edges: &[Value],
    max_files: usize,
) -> (Vec<&'a Node>, usize) {
    let ids: BTreeSet<_> = targets.iter().map(|n| n.id.as_str()).collect();
    let mut candidates = BTreeMap::new();
    for edge in edges.iter().filter(|e| e["kind"] == "calls") {
        let source = edge["source"].as_str().unwrap_or("");
        let target = edge["target"].as_str().unwrap_or("");
        let related = if ids.contains(source) {
            Some((target, 0))
        } else if ids.contains(target) {
            Some((source, 1))
        } else {
            None
        };
        if let Some((id, direction)) = related {
            if !ids.contains(id) {
                if let Some(node) = nodes.get(id) {
                    let test = node
                        .file_path
                        .split('/')
                        .any(|p| matches!(p, "tests" | "test"))
                        || node.name.starts_with("test_");
                    candidates
                        .entry((
                            test,
                            direction,
                            node.file_path.as_str(),
                            node.start_line,
                            id,
                        ))
                        .or_insert(node);
                }
            }
        }
    }
    let mut files: BTreeSet<_> = targets.iter().map(|n| n.file_path.as_str()).collect();
    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    // Round-robin files prevents local guards/helpers from consuming every slot
    // before a cross-file part of the flow. Production still precedes tests.
    let mut per_file = BTreeMap::<&str, usize>::new();
    for test_pass in [false, true] {
        for quota in 1..=4 {
            for ((is_test, _, _, _, _), node) in &candidates {
                if *is_test != test_pass || seen.contains(&node.id) || selected.len() >= 4 {
                    continue;
                }
                if per_file.get(node.file_path.as_str()).copied().unwrap_or(0) >= quota {
                    continue;
                }
                if !files.contains(node.file_path.as_str()) && files.len() >= max_files {
                    continue;
                }
                files.insert(node.file_path.as_str());
                *per_file.entry(node.file_path.as_str()).or_default() += 1;
                seen.insert(node.id.clone());
                selected.push(*node);
            }
        }
    }
    let omitted = candidates
        .values()
        .map(|n| &n.id)
        .collect::<BTreeSet<_>>()
        .len()
        .saturating_sub(selected.len());
    (selected, omitted)
}

fn node(row: &Row<'_>, offset: usize) -> rusqlite::Result<Node> {
    Ok(Node {
        id: row.get(offset)?,
        name: row.get(offset + 1)?,
        kind: row.get(offset + 2)?,
        file_path: row.get(offset + 3)?,
        start_line: row.get(offset + 4)?,
    })
}

fn validate_node(project: &Project, item: &mut Node) -> Result<(), Failure> {
    if [&item.id, &item.name, &item.kind]
        .iter()
        .any(|s| s.is_empty() || s.len() > 1024 || s.chars().any(char::is_control))
        || item.file_path.len() > 1000
        || item.file_path.chars().any(char::is_control)
        || item.start_line <= 0
        || item.start_line > 10_000_000
    {
        return Err(CONTRACT);
    }
    item.file_path = item.file_path.replace('\\', "/");
    // Also reject metadata/credential paths and validate existing ancestors for
    // stale/deleted indexed files. Never echo an unchecked path from the index.
    if super::project::external_restricted(&item.file_path)
        || super::project::external_sensitive(&item.file_path)
        || item
            .file_path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(CONTRACT);
    }
    project
        .target_optional(&item.file_path)
        .map_err(|_| CONTRACT)?;
    Ok(())
}

fn identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c == '$' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c == '$' || c.is_alphanumeric())
}

fn query_symbols(args: &ExploreArgs) -> Vec<&str> {
    if let Some(symbol) = args.symbol.as_deref() {
        return vec![symbol];
    }
    args.query
        .split_whitespace()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn razor_file_selector(args: &ExploreArgs) -> bool {
    args.query.trim().is_empty()
        && args.symbol.is_none()
        && args.symbol_id.is_none()
        && args.container.is_none()
        && args.view != Some(ExploreView::Impact)
        && args.file_path.as_deref().is_some_and(|path| {
            std::path::Path::new(path).extension().and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("cshtml"))
        })
}

fn validate_args(args: &ExploreArgs) -> Result<(), Failure> {
    let symbols = query_symbols(args);
    let max_files = args.max_files.unwrap_or(4);
    let max_chars = args.max_chars.unwrap_or(12000);
    let max_depth = args.max_depth.unwrap_or(1);
    let impact = args.view == Some(ExploreView::Impact);
    if impact
        && (args.file_path.as_ref().is_none_or(|file| file.is_empty())
            || (args.symbol.is_none() && args.symbol_id.is_none()))
    {
        return Err((
            "ARGUMENT",
            "view=impact requires filePath and symbol (or symbolId)",
        ));
    }
    if !(1..=100).contains(&args.limit.unwrap_or(20)) {
        return Err(("ARGUMENT", "limit must be between 1 and 100"));
    }
    if args.query.len() > 2000
        || args
            .file_path
            .as_ref()
            .is_some_and(|p| p.len() > 4096 || p.contains('\0'))
        || args
            .container
            .as_ref()
            .is_some_and(|p| p.is_empty() || p.len() > 1000 || p.contains('\0'))
        || args
            .symbol_id
            .as_ref()
            .is_some_and(|p| p.is_empty() || p.len() > 1024 || p.contains('\0'))
        || args.query.contains('\0')
        || (symbols.is_empty() && args.symbol_id.is_none() && !razor_file_selector(args))
        || symbols.len() > 4
        || symbols
            .iter()
            .any(|symbol| symbol.len() > 256 || !identifier(symbol))
        || !(1..=4).contains(&max_files)
        || !(1000..=16000).contains(&max_chars)
        || !(1..=4).contains(&max_depth)
    {
        return Err((
            "ARGUMENT",
            "Explore requires symbol or 1..4 whitespace-separated identifiers; maxFiles 1..4, maxChars 1000..16000, maxDepth 1..4",
        ));
    }
    Ok(())
}

fn sync_failure(report: Value, code: &str, message: &str) -> Value {
    json!({"ok":false,"cancelled":code=="CANCELLED","timedOut":code=="TIMEOUT",
        "diagnostics":[{"code":code,"message":message}],"sync":report,
        "explorationPerformed":false,"globalFreshnessCertified":false})
}

pub(super) fn synchronized(
    project: &Project,
    args: &ExploreArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    validate_args(args)?;
    if let Some(file) = &args.file_path {
        project.target(file)?;
    }
    let report = super::index_store::sync_workspace_all(&project.root, context)?;
    if report["selectedScopeApplied"] != true
        || report["discoveryComplete"] != true
        || report["uncheckedFiles"] != 0
        || report["unsupportedChangedFiles"] != 0
    {
        let code = report["failure"]["code"]
            .as_str()
            .unwrap_or("SYNC_INCOMPLETE")
            .to_owned();
        return Ok(sync_failure(
            report,
            &code,
            "Index update incomplete; inspect confirmed commits before retrying",
        ));
    }
    match explore(project, args, context) {
        Ok(mut result) => {
            // Policy exclusions are not failed work. Keep queried-file checks.
            if report["restrictedIndexedFiles"].as_u64().unwrap_or(0) > 0 {
                let warning = json!({"code":"SYNC_RESTRICTED_SKIPPED","severity":"warning",
                    "message":"Restricted index entries were excluded by policy; queried-file checks still apply. Global index freshness is not certified.",
                    "retryable":false});
                if let Some(diagnostics) = result["diagnostics"].as_array_mut() {
                    diagnostics.push(warning);
                } else {
                    result["diagnostics"] = json!([warning]);
                }
            }
            result["sync"] = report;
            Ok(result)
        }
        Err((code, message)) => Ok(sync_failure(report, code, message)),
    }
}

pub(super) fn explore(
    project: &Project,
    args: &ExploreArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let diagnostic = std::cell::RefCell::new(json!({
        "databasePath": project.root.join(".codegraph/codegraph.db"),
        "stage": "filesystem", "expectedSchemaVersion": {"min":1,"max":11},
        "globalFreshnessCertified": false
    }));
    match explore_inner(project, args, context, &diagnostic) {
        Err(("INDEX", message)) => Ok(json!({
            "ok":false, "explorationPerformed":false,
            "diagnostics":[{"code":"INDEX","message":message}],
            "indexDiagnostic":diagnostic.into_inner()
        })),
        result => result,
    }
}

fn explore_inner(
    project: &Project,
    args: &ExploreArgs,
    context: &OperationContext,
    diagnostic: &std::cell::RefCell<Value>,
) -> Result<Value, Failure> {
    context.check()?;
    validate_args(args)?;
    if let Some(file) = &args.file_path {
        project.target(file)?;
    }
    let impact = args.view == Some(ExploreView::Impact);
    let symbol = args.symbol.as_deref().unwrap_or(args.query.trim());
    let max_files = if impact {
        1
    } else {
        args.max_files.unwrap_or(4)
    };
    let max_depth = args.max_depth.unwrap_or(1);
    let direction = if impact {
        ExploreDirection::Inbound
    } else {
        args.direction.unwrap_or(ExploreDirection::Both)
    };
    let summary = impact || args.view == Some(ExploreView::Summary);
    let max_chars = if impact {
        1000
    } else if summary {
        args.max_chars.unwrap_or(2000)
    } else {
        args.max_chars.unwrap_or(12000)
    };
    let path = project
        .target_optional(".codegraph/codegraph.db")
        .map_err(|_| {
            diagnostic.borrow_mut()["cause"] = json!("database_path_rejected");
            INDEX
        })?;
    let metadata = std::fs::metadata(&path).map_err(|error| {
        diagnostic.borrow_mut()["cause"] = json!(match error.kind() {
            std::io::ErrorKind::NotFound => "database_missing",
            std::io::ErrorKind::PermissionDenied => "permission_denied",
            _ => "metadata_unavailable",
        });
        INDEX
    })?;
    if !metadata.is_file() || metadata.len() > 512 * 1024 * 1024 {
        diagnostic.borrow_mut()["cause"] = json!("database_type_or_size_rejected");
        return Err(INDEX);
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        project
            .target_optional(&format!(".codegraph/codegraph.db{suffix}"))
            .map_err(|_| INDEX)?;
    }
    #[cfg(windows)]
    let _pins = super::edit_commit::pin_directory(project, path.parent().ok_or(INDEX)?)?;
    let started = Instant::now();
    let failed = |error: rusqlite::Error| {
        let mut details = diagnostic.borrow_mut();
        if let rusqlite::Error::SqliteFailure(code, _) = &error {
            details["sqliteCode"] = json!(format!("{:?}", code.code));
            details["sqliteExtendedCode"] = json!(code.extended_code);
            details["cause"] = json!(match code.code {
                rusqlite::ErrorCode::DatabaseBusy => "database_busy",
                rusqlite::ErrorCode::DatabaseLocked => "database_locked",
                rusqlite::ErrorCode::DatabaseCorrupt => "database_corrupt",
                rusqlite::ErrorCode::NotADatabase => "not_a_database",
                rusqlite::ErrorCode::PermissionDenied | rusqlite::ErrorCode::CannotOpen =>
                    "database_inaccessible",
                _ => "sqlite_operation_failed",
            });
        } else {
            details["cause"] = json!("schema_or_value_contract_mismatch");
        }
        context.check().err().unwrap_or_else(|| {
            if started.elapsed() >= Duration::from_secs(5) {
                BUDGET
            } else {
                INDEX
            }
        })
    };
    diagnostic.borrow_mut()["stage"] = json!("sqlite_open");
    let db = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(failed)?;
    db.busy_timeout(Duration::from_millis(100))
        .map_err(failed)?;
    db.set_limit(Limit::SQLITE_LIMIT_LENGTH, 65536)
        .map_err(failed)?;
    db.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 8192)
        .map_err(failed)?;
    let cancel = context.clone();
    db.progress_handler(
        1000,
        Some(move || cancel.check().is_err() || started.elapsed() >= Duration::from_secs(5)),
    )
    .map_err(failed)?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-4096; BEGIN;").map_err(failed)?;
    // Refuse views and virtual-table substitutions; this adapter supports only
    // the installed CodeGraph table contract, not arbitrary SQLite databases.
    diagnostic.borrow_mut()["stage"] = json!("schema_validation");
    let tables: i64 = db.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('schema_versions','nodes','edges') AND type='table' AND upper(sql) LIKE 'CREATE TABLE%'", [], |r| r.get(0)).map_err(failed)?;
    if tables != 3 {
        diagnostic.borrow_mut()["cause"] = json!("required_tables_missing_or_unsupported");
        return Err(INDEX);
    }
    let version: i64 = db
        .query_row("SELECT max(version) FROM schema_versions", [], |r| r.get(0))
        .map_err(failed)?;
    diagnostic.borrow_mut()["actualSchemaVersion"] = json!(version);
    if !(1..=11).contains(&version) {
        diagnostic.borrow_mut()["cause"] = json!("unsupported_schema_version");
        return Err(INDEX);
    }
    let has_qualified: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('nodes') WHERE name='qualified_name')",
            [],
            |r| r.get(0),
        )
        .map_err(failed)?;
    if args.container.is_some() && !has_qualified {
        return Err((
            "ARGUMENT",
            "This legacy index lacks qualified_name; select with filePath or symbolId",
        ));
    }
    diagnostic.borrow_mut()["stage"] = json!("graph_query");
    let query = if has_qualified {
        "SELECT id,name,kind,file_path,start_line FROM nodes WHERE (name IN (SELECT value FROM json_each(?1)) COLLATE BINARY OR (?1='[]' AND ?3 IS NOT NULL) OR (?5 AND kind='file')) AND (?2 IS NULL OR file_path=?2 COLLATE BINARY) AND (?3 IS NULL OR id=?3 COLLATE BINARY) AND (?4 IS NULL OR qualified_name=?4||'::'||name COLLATE BINARY OR qualified_name=?4||'.'||name COLLATE BINARY) ORDER BY file_path,start_line,id LIMIT 101"
    } else {
        "SELECT id,name,kind,file_path,start_line FROM nodes WHERE (name IN (SELECT value FROM json_each(?1)) COLLATE BINARY OR (?1='[]' AND ?3 IS NOT NULL) OR (?5 AND kind='file')) AND (?2 IS NULL OR file_path=?2 COLLATE BINARY) AND (?3 IS NULL OR id=?3 COLLATE BINARY) AND ?4 IS NULL ORDER BY file_path,start_line,id LIMIT 101"
    };
    let mut search = db.prepare(query).map_err(failed)?;
    let symbol_names = serde_json::to_string(&query_symbols(args)).map_err(|_| CONTRACT)?;
    let mut hits = search
        .query_map(
            rusqlite::params![symbol_names, args.file_path, args.symbol_id, args.container, razor_file_selector(args)],
            |r| node(r, 0),
        )
        .map_err(failed)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(failed)?;
    if hits.is_empty()
        && (args.file_path.is_some() || args.container.is_some() || args.symbol_id.is_some())
    {
        return Ok(
            json!({"ok":false,"candidates":[],"diagnostics":[{"code":"TARGET_NOT_RESOLVED","message":"No indexed target matches all selectors; inspect outline or request candidates without the conflicting selector"}],"_complete":false}),
        );
    }
    let mut truncated = hits.len() > 100;
    hits.truncate(100);
    let symbol = if symbol.is_empty() {
        hits.first().map(|hit| hit.name.clone()).unwrap_or_default()
    } else {
        symbol.to_owned()
    };
    let candidates: Vec<_> = hits.iter().take(20).cloned().collect();
    let candidates_omitted = hits.len().saturating_sub(candidates.len());
    let mut selection_limited = truncated;
    let mut per_node_edges_limited = false;
    let mut total_edges_limited = false;
    let mut definitions_limited = false;
    let mut files = BTreeSet::new();
    let mut targets = Vec::new();
    for mut hit in hits {
        context.check()?;
        validate_node(project, &mut hit)?;
        if !files.contains(&hit.file_path) && files.len() >= max_files || targets.len() >= 16 {
            truncated = true;
            selection_limited = true;
            continue;
        }
        files.insert(hit.file_path.clone());
        targets.push(hit);
    }
    let mut nodes: BTreeMap<String, Node> =
        targets.iter().map(|n| (n.id.clone(), n.clone())).collect();
    let mut edges = Vec::new();
    let mut edge_ids = BTreeSet::new();
    let mut export_contributors = BTreeMap::new();
    // Minimal legacy reader schemas need not have edge metadata. Detect the
    // column explicitly; never turn a malformed full-schema query into a
    // silent "no provenance" fallback.
    let has_metadata: i64 = db
        .query_row(
            "SELECT count(*) FROM pragma_table_info('edges') WHERE name='metadata'",
            [],
            |r| r.get(0),
        )
        .map_err(failed)?;
    let mut export_query = if has_metadata == 1 {
        Some(db.prepare("SELECT CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.exportContributors') END FROM edges WHERE id=?1").map_err(failed)?)
    } else {
        None
    };
    let directions: &[bool] = match direction {
        ExploreDirection::Inbound => &[true],
        ExploreDirection::Outbound => &[false],
        ExploreDirection::Both => &[true, false],
    };
    let mut frontier = VecDeque::new();
    let mut depths: BTreeMap<String, BTreeMap<&str, usize>> = BTreeMap::new();
    for target in &targets {
        for &incoming in directions {
            let label = if incoming { "inbound" } else { "outbound" };
            let previous = depths
                .entry(target.id.clone())
                .or_default()
                .insert(label, 0);
            if previous.is_none() {
                frontier.push_back((target.id.clone(), 0, incoming));
            }
        }
    }
    let mut expanded = 0;
    let mut depth_boundary = false;
    // Each direction has its own BFS frontier. Inbound traversal never turns
    // outbound at an intermediate node (which would introduce sibling callers).
    'traversal: while let Some((current, depth, incoming)) = frontier.pop_front() {
        if depth >= max_depth {
            depth_boundary = true;
            continue;
        }
        context.check()?;
        if started.elapsed() >= Duration::from_secs(5) {
            return Err(BUDGET);
        }
        expanded += 1;
        // SQL text and edge kinds are closed constants, never request text.
        let sql = if incoming {
            "SELECT e.id,e.source,e.target,e.kind,n.id,n.name,n.kind,n.file_path,n.start_line FROM edges e JOIN nodes n ON n.id=e.source WHERE e.target=?1 AND e.kind IN ('calls','references','imports','instantiates','navigates') ORDER BY e.id LIMIT 21"
        } else {
            "SELECT e.id,e.source,e.target,e.kind,n.id,n.name,n.kind,n.file_path,n.start_line FROM edges e JOIN nodes n ON n.id=e.target WHERE e.source=?1 AND e.kind IN ('calls','references','imports','instantiates','navigates') ORDER BY e.id LIMIT 21"
        };
        let mut statement = db.prepare(sql).map_err(failed)?;
        let mut rows = statement.query([&current]).map_err(failed)?;
        let mut count = 0;
        while let Some(row) = rows.next().map_err(failed)? {
            context.check()?;
            count += 1;
            if started.elapsed() >= Duration::from_secs(5) {
                return Err(BUDGET);
            }
            if count > 20 {
                truncated = true;
                per_node_edges_limited = true;
                break;
            }
            let id: i64 = row.get(0).map_err(failed)?;
            let source: String = row.get(1).map_err(failed)?;
            let destination: String = row.get(2).map_err(failed)?;
            let kind: String = row.get(3).map_err(failed)?;
            let mut related = node(row, 4).map_err(failed)?;
            validate_node(project, &mut related)?;
            if source.len() > 1024 || destination.len() > 1024 {
                return Err(CONTRACT);
            }
            if !edge_ids.contains(&id) && edges.len() >= 160 {
                truncated = true;
                total_edges_limited = true;
                break 'traversal;
            }
            let label = if incoming { "inbound" } else { "outbound" };
            let known = depths.entry(related.id.clone()).or_default();
            if let std::collections::btree_map::Entry::Vacant(entry) = known.entry(label) {
                entry.insert(depth + 1);
                depth_boundary |= depth + 1 == max_depth;
                frontier.push_back((related.id.clone(), depth + 1, incoming));
            }
            nodes.insert(related.id.clone(), related);
            if edge_ids.insert(id) {
                let provenance = export_query
                    .as_mut()
                    .map(|query| query.query_row([id], |r| r.get::<_, Option<String>>(0)))
                    .transpose()
                    .map_err(failed)?
                    .flatten();
                super::index_freshness::export_contributors(
                    provenance.as_deref(),
                    &mut export_contributors,
                )?;
                edges.push(json!({"id":id,"source":source,"target":destination,"kind":kind}));
            }
        }
    }
    let mut contributing_files = nodes
        .values()
        .map(|node| node.file_path.clone())
        .collect::<BTreeSet<_>>();
    contributing_files.extend(export_contributors.keys().cloned());
    let mut indexed_files =
        super::index_freshness::indexed_files(&db, contributing_files, context)?;
    for (path, digest) in export_contributors {
        let expected = indexed_files.get_mut(&path).ok_or(CONTRACT)?;
        if expected.hash.as_deref() != Some(digest.as_str()) {
            expected.hash = Some(digest);
            expected.reason = Some("export_provenance_mismatch");
        }
    }
    db.execute_batch("ROLLBACK").map_err(failed)?;
    context.check()?;
    if started.elapsed() >= Duration::from_secs(5) {
        return Err(BUDGET);
    }
    let assessment = super::index_freshness::Assessment::start(project, indexed_files, context)?;
    let mut definitions = Vec::new();
    let mut delivered_symbols = BTreeSet::new();
    let mut unresolved = Vec::new();
    for name in query_symbols(args) {
        if !targets.iter().any(|n| n.name == name) {
            unresolved.push(json!({"symbol":name,"role":"target","code":"TARGET_NOT_RESOLVED","message":"No selected indexed target; not proof of global absence"}));
        }
    }
    let mut text = if summary {
        format!("Símbolo: {symbol}\nRelaciones indexadas; vigencia no certificada.\n")
    } else {
        presentation::header(&symbol)
    };
    let (related_context, context_omitted) = if summary {
        (Vec::new(), 0)
    } else {
        context_nodes(&targets, &nodes, &edges, max_files)
    };
    let related_ids: BTreeSet<_> = related_context.iter().map(|n| n.id.as_str()).collect();
    let context_targets: Vec<_> = targets
        .iter()
        .chain(related_context.iter().copied())
        .collect();
    for selected in &context_targets {
        // File targets anchor render/helper relationships; they have no local method body.
        if selected.kind == "file" && razor_file_selector(args) {
            continue;
        }
        let file = &selected.file_path;
        let related = related_ids.contains(selected.id.as_str());
        context.check()?;
        let read = symbols::read_symbol(
            project,
            &ReadSymbolArgs {
                external_scope: None,
                path: file.clone(),
                symbol: selected.name.clone(),
                container: if related {
                    None
                } else {
                    args.container.clone()
                },
                symbol_id: None,
                view: if summary {
                    Some(harness_domain::contracts::SymbolView::Signature)
                } else {
                    None
                },
                session: None,
            },
            context,
        );
        match read {
            Ok(value) => {
                let identity = (file.clone(), value["hash"].to_string(), value["symbol"]["symbolId"].to_string());
                if !delivered_symbols.insert(identity) { continue; }
                let body = if summary { value["symbol"]["signature"].as_str().unwrap_or("") } else { value["text"].as_str().or_else(|| value["_detail"]["text"].as_str()).unwrap_or("") };
                let remaining = 64000usize.saturating_sub(utf16_len(&text));
                let body_budget = if summary { 512 } else if related { 2000 } else { (max_chars / targets.len().max(1)).max(1000).min(16000) };
                let preview = prefix(body, remaining.min(body_budget));
                truncated |= utf16_len(body) > utf16_len(preview);
                definitions_limited |= utf16_len(body) > utf16_len(preview);
                if summary {
                    text.push_str(&format!("\n### {file}\n{preview}\n"));
                } else {
                    text.push_str(&presentation::source(file, value["symbol"]["start"].as_u64().unwrap_or(1), preview, utf16_len(body) > utf16_len(preview)));
                }
                definitions.push(json!({"path":file,"hash":value["hash"],"symbol":value["symbol"],"role":if related {"related"} else {"target"},
                    "_bodyComplete":!summary && utf16_len(body) == utf16_len(preview),"_textEnd":utf16_len(&text)}));
            }
            Err((code @ ("CANCELLED" | "TIMEOUT"), message)) => return Err((code, message)),
            Err((code, _)) => unresolved.push(json!({"path":file,"symbol":selected.name,"role":if related {"related"} else {"target"},"code":code,"message":"Indexed candidate not resolved by the current local parser; use outline/read_symbol with an exact selector"})),
        }
    }
    if !summary && context_omitted > 0 {
        text.push_str(&format!("\n> {context_omitted} related symbols omitted by the 4-symbol/file context budget; select a name from graph and call read_symbol.\n"));
    }
    text.push_str(if summary {
        "\nRelaciones indexadas: ver graph.relations; grafo completo en detail.ref.\n"
    } else {
        "\n**Call paths / blast radius — indexed relationships**\n\nNot execution traces or exhaustive impact; current-source validity is reported in fileFreshness.\n"
    });
    let mut relation_keys = BTreeSet::new();
    let mut visible_edges: Vec<_> = edges
        .iter()
        .filter(|e| {
            relation_keys.insert((
                e["source"].to_string(),
                e["target"].to_string(),
                e["kind"].to_string(),
            ))
        })
        .collect();
    visible_edges.sort_by_key(|e| {
        (
            e["kind"] != "calls",
            e["source"].to_string(),
            e["target"].to_string(),
        )
    });
    if !summary {
        for selected in &targets {
            let callers: BTreeSet<_> = visible_edges
                .iter()
                .filter(|e| e["kind"] == "calls" && e["target"] == selected.id)
                .filter_map(|e| e["source"].as_str())
                .collect();
            let tests: BTreeSet<_> = callers
                .iter()
                .filter_map(|id| nodes.get(*id))
                .filter(|n| {
                    n.file_path
                        .split('/')
                        .any(|p| matches!(p, "tests" | "test"))
                        || n.name.starts_with("test_")
                })
                .map(|n| n.file_path.as_str())
                .collect();
            text.push_str(&format!(
                "- {}: {} unique direct callers in returned graph; test files: {}.\n",
                selected.name,
                callers.len(),
                if tests.is_empty() {
                    "none identified (not proof of absence)".to_owned()
                } else {
                    tests.into_iter().collect::<Vec<_>>().join(", ")
                }
            ));
        }
        text.push_str("\n**Relationships (calls first)**\n");
    }
    for edge in visible_edges.iter().take(if summary { 0 } else { 20 }) {
        let source = &nodes[edge["source"].as_str().ok_or(CONTRACT)?];
        let target = &nodes[edge["target"].as_str().ok_or(CONTRACT)?];
        text.push_str(&format!(
            "- {} ({}:{}) → {} ({}:{}) [{}]\n",
            source.name,
            source.file_path,
            source.start_line,
            target.name,
            target.file_path,
            target.start_line,
            edge["kind"].as_str().ok_or(CONTRACT)?
        ));
    }
    if !summary && visible_edges.len() > 20 {
        text.push_str(&format!(
            "\n> {} additional relationships omitted from text; available in graph/detail.\n",
            visible_edges.len() - 20
        ));
    }
    if !summary && (truncated || !unresolved.is_empty()) {
        text.push_str("\n> Partial context: inspect truncation and unresolved before drawing absence conclusions.\n");
    }
    let definition_hashes = definitions
        .iter()
        .filter_map(|d| {
            Some((
                d["path"].as_str()?.to_string(),
                d["hash"].as_str()?.to_string(),
            ))
        })
        .collect();
    let freshness = assessment.finish(project, &definition_hashes, context)?;
    let matching = freshness["status"] == "matches_observed_files";
    if args.require_matching_files == Some(true) && !matching {
        return Err(if freshness["status"] == "stale" {
            (
                "INDEX_STALE",
                "Contributing source files changed since indexing; synchronize the index before requiring matching files",
            )
        } else {
            (
                "INDEX_UNCHECKED",
                "Contributing source file matches could not be established within coverage and read budgets",
            )
        });
    }
    let diagnostics = if matching {
        vec![]
    } else {
        vec![
            json!({"code":if freshness["status"] == "stale" {"INDEX_STALE"} else {"INDEX_UNCHECKED"},"severity":"warning","message":"Contributing file freshness is stale or unverified; indexed relationships must not be treated as current source evidence"}),
        ]
    };
    let preview = prefix(&text, max_chars);
    let partial = truncated
        || context_omitted > 0
        || !unresolved.is_empty()
        || targets.is_empty()
        || !matching;
    let traversal = json!({"direction":direction,"maxDepth":max_depth,"nodeDepths":depths,"expandedNodes":expanded,"depthBoundaryReached":depth_boundary,
        "limits":{"edges":160,"nodes":176,"edgesPerNodeDirection":20},"semantics":"independent directed breadth-first traversals from selected exact-name targets; depths are minimum discovered hops within scanned edges per direction, not global shortest paths"});
    // Graph coverage must not depend on the amount of source rendered for the client.
    // Reaching the requested depth is reported separately, not proof of dropped edges.
    let graph_limited = selection_limited || per_node_edges_limited || total_edges_limited;
    let truncation = json!({"targetSelection":selection_limited,"depthBoundary":depth_boundary,"perNodeEdges":per_node_edges_limited,"totalEdges":total_edges_limited});
    let graph = json!({"format":"codegraph-sqlite-readonly-v1","schemaVersion":version,"files":files,"nodes":nodes.values().collect::<Vec<_>>(),"edges":edges,"truncated":graph_limited,"truncation":truncation,"traversal":traversal});
    let displayed_graph = if summary {
        let mut kinds = BTreeMap::<String, usize>::new();
        for edge in &edges {
            *kinds
                .entry(edge["kind"].as_str().unwrap_or("unknown").into())
                .or_default() += 1;
        }
        let mut relations = Vec::new();
        let mut shown = BTreeSet::new();
        for incoming in [true, false] {
            for edge in edges
                .iter()
                .filter(|edge| {
                    targets.iter().any(|n| {
                        if incoming {
                            edge["target"] == n.id
                        } else {
                            edge["source"] == n.id
                        }
                    })
                })
                .take(4)
            {
                if !shown.insert(edge["id"].as_i64().ok_or(CONTRACT)?) {
                    continue;
                }
                let source = &nodes[edge["source"].as_str().ok_or(CONTRACT)?];
                let target = &nodes[edge["target"].as_str().ok_or(CONTRACT)?];
                relations.push(json!({"source":{"name":source.name,"filePath":source.file_path,"line":source.start_line},"target":{"name":target.name,"filePath":target.file_path,"line":target.start_line},"kind":edge["kind"]}));
            }
        }
        let omitted_relations = edges.len().saturating_sub(relations.len());
        json!({"format":"codegraph-summary-v1","countsScope":"returned-subgraph-not-global", "nodeCount":nodes.len(),"edgeCount":edges.len(),"edgeKinds":kinds,
            "relations":relations,"relationsOmitted":omitted_relations,"relationsOmittedScope":"summary_display_only","summaryTruncated":omitted_relations>0,
            "targetIds":targets.iter().map(|n| &n.id).collect::<Vec<_>>(),"truncated":graph_limited,"truncation":truncation,
            "direction":direction,"maxDepth":max_depth,"fullGraph":"detail.ref -> graph; use data_query resultRef for projection"})
    } else {
        graph.clone()
    };
    let context_truncation = json!({"definitionBodies":definitions_limited,"textPreview":utf16_len(&text)>max_chars,
        "relatedBodies":context_omitted>0,"summaryRelations":displayed_graph["summaryTruncated"]==true});
    let displayed_candidates: Vec<_> = candidates
        .iter()
        .take(if summary { 5 } else { 20 })
        .collect();
    let displayed_candidates_omitted =
        candidates_omitted + candidates.len() - displayed_candidates.len();
    for definition in &mut definitions {
        let complete = definition["_bodyComplete"] == true
            && definition["_textEnd"]
                .as_u64()
                .is_some_and(|end| end <= max_chars as u64);
        definition.as_object_mut().unwrap().remove("_bodyComplete");
        definition.as_object_mut().unwrap().remove("_textEnd");
        definition["sourceStatus"] = json!(if complete {
            "complete"
        } else if summary {
            "signature_only"
        } else {
            "partial"
        });
        if !complete {
            let mut arguments = json!({"path":definition["path"],"symbol":definition["symbol"]["name"],
                "symbolId":definition["symbol"]["symbolId"],"view":"full"});
            if let Some(session) = &args.session {
                arguments["session"] = json!(session);
            }
            arguments["action"] = json!("read");
            definition["nextCall"] = json!({"tool":"harness_symbols","arguments":arguments});
        }
    }
    Ok(
        json!({"text":preview,"focus":{"symbol":symbol,"status":if definitions.iter().any(|d| d["role"] == "target") || (razor_file_selector(args) && !targets.is_empty()) {"resolved"} else {"unresolved"}},
        "view":if impact {"impact"} else if summary {"summary"} else {"full"},"definitionView":if summary {"signature"} else {"full"},"editReady":false,"contextTruncation":context_truncation,
        "reuseGuidance":"Use complete definitions already present in your context; do not read them again unless missing or changed. For partial/signature_only definitions use nextCall only when the missing body is needed. Hashes identify the observed source, not future freshness. Numbered Markdown is not replacement text.",
        "contextSelection":{"requestedSymbols":query_symbols(args),"relatedReturned":definitions.iter().filter(|d| d["role"] == "related").count(),"relatedOmitted":context_omitted,"maxRelatedSymbols":4,"maxFiles":max_files,"policy":"direct calls; production before tests; ambiguous local bodies are not guessed"},
        "definitions":definitions,"unresolved":unresolved,"candidates":displayed_candidates,"candidatesOmitted":displayed_candidates_omitted,"selection":{"filePath":args.file_path,"container":args.container,"symbolId":args.symbol_id,"multipleTargets":targets.len()>1,"symbolIdNamespace":"codegraph-node-id"},"graph":displayed_graph,"relationshipSource":"codegraph-index",
        "indexFreshness":"not_certified","synchronized":false,"absenceProven":false,
        "fileFreshness":freshness,"diagnostics":diagnostics,
        "synchronization":{"performed":false,"nativeIndexerAvailable":true,"supportedLanguages":["rust","csharp","javascript","typescript","jsx","tsx"],"reason":"syncIndex_not_requested"},
        "notice":"Bounded indexed relationships, not execution traces. Local symbol readers support Rust, JavaScript and TypeScript; other bodies may be unresolved. graph.truncated concerns graph selection/edges only; contextTruncation concerns presentation. No exhaustive impact, absence or index freshness claims. Indexer availability does not grant write authorization. Preview is not replacement text.",
        "truncated":truncated || utf16_len(&text)>max_chars || context_omitted>0 || displayed_graph["summaryTruncated"]==true,"_complete":!partial,
        "_detail":{"text":text,"definitionView":if summary {"signature"} else {"full"},"graph":graph,"contextTruncation":context_truncation,"candidates":candidates,"candidatesOmitted":candidates_omitted,"definitions":definitions,"unresolved":unresolved,"fileFreshness":freshness,"indexFreshness":"not_certified","synchronized":false}}),
    )
}
