//! Read existing CodeGraph SQLite indexes without Node, migrations or arbitrary SQL.
use super::{project::Project, symbols};
use harness_domain::{
    Failure,
    contracts::{ExploreArgs, ExploreDirection, ReadSymbolArgs},
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
    "CodeGraph index unavailable, unsupported, busy or invalid; no raw database diagnostics returned",
);
const CONTRACT: Failure = (
    "GRAPH_CONTRACT",
    "CodeGraph returned invalid or restricted graph metadata",
);
const BUDGET: Failure = (
    "GRAPH_LIMIT",
    "CodeGraph query exceeded its bounded read budget",
);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Node {
    id: String,
    name: String,
    kind: String,
    file_path: String,
    start_line: i64,
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

fn validate_args(args: &ExploreArgs) -> Result<(), Failure> {
    let symbol = args.symbol.as_deref().unwrap_or(args.query.trim());
    let max_files = args.max_files.unwrap_or(4);
    let max_chars = args.max_chars.unwrap_or(12000);
    let max_depth = args.max_depth.unwrap_or(1);
    if args.query.len() > 2000
        || args.query.contains('\0')
        || symbol.len() > 256
        || !identifier(symbol)
        || !(1..=4).contains(&max_files)
        || !(1000..=16000).contains(&max_chars)
        || !(1..=4).contains(&max_depth)
    {
        return Err((
            "ARGUMENT",
            "Explore requires symbol or a simple-identifier query; maxFiles 1..4, maxChars 1000..16000, maxDepth 1..4",
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
    let report = super::index_store::sync_workspace_all(&project.root, context)?;
    if report["selectedScopeApplied"] != true
        || report["discoveryComplete"] != true
        || report["uncheckedFiles"] != 0
        || report["unsupportedChangedFiles"] != 0
        || report["restrictedIndexedFiles"] != 0
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
    context.check()?;
    validate_args(args)?;
    let symbol = args.symbol.as_deref().unwrap_or(args.query.trim());
    let max_files = args.max_files.unwrap_or(4);
    let max_chars = args.max_chars.unwrap_or(12000);
    let max_depth = args.max_depth.unwrap_or(1);
    let direction = args.direction.unwrap_or(ExploreDirection::Both);
    let path = project
        .target(".codegraph/codegraph.db")
        .map_err(|_| INDEX)?;
    let metadata = std::fs::metadata(&path).map_err(|_| INDEX)?;
    if !metadata.is_file() || metadata.len() > 512 * 1024 * 1024 {
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
    let failed = |_| {
        context.check().err().unwrap_or_else(|| {
            if started.elapsed() >= Duration::from_secs(5) {
                BUDGET
            } else {
                INDEX
            }
        })
    };
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
    let tables: i64 = db.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('schema_versions','nodes','edges') AND type='table' AND upper(sql) LIKE 'CREATE TABLE%'", [], |r| r.get(0)).map_err(failed)?;
    if tables != 3 {
        return Err(INDEX);
    }
    let version: i64 = db
        .query_row("SELECT max(version) FROM schema_versions", [], |r| r.get(0))
        .map_err(failed)?;
    if !(1..=11).contains(&version) {
        return Err(INDEX);
    }
    let mut search = db.prepare("SELECT id,name,kind,file_path,start_line FROM nodes WHERE name=?1 COLLATE BINARY ORDER BY file_path,start_line,id LIMIT 101").map_err(failed)?;
    let mut hits = search
        .query_map([symbol], |r| node(r, 0))
        .map_err(failed)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(failed)?;
    let mut truncated = hits.len() > 100;
    hits.truncate(100);
    let mut files = BTreeSet::new();
    let mut targets = Vec::new();
    for mut hit in hits {
        context.check()?;
        validate_node(project, &mut hit)?;
        if !files.contains(&hit.file_path) && files.len() >= max_files || targets.len() >= 16 {
            truncated = true;
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
    let mut unresolved = Vec::new();
    let mut text = format!(
        "Símbolo: {symbol}\nRelaciones de índice CodeGraph existente; vigencia no certificada.\n"
    );
    for file in &files {
        context.check()?;
        let read = symbols::read_symbol(
            project,
            &ReadSymbolArgs {
                external_scope: None,
                path: file.clone(),
                symbol: symbol.into(),
                container: None,
                symbol_id: None,
                view: None,
                session: None,
            },
            context,
        );
        match read {
            Ok(value) => {
                let body = value["text"].as_str().or_else(|| value["_detail"]["text"].as_str()).unwrap_or("");
                let remaining = 64000usize.saturating_sub(utf16_len(&text));
                let preview = prefix(body, remaining.min(16000));
                truncated |= utf16_len(body) > utf16_len(preview);
                text.push_str(&format!("\n### {file}\n{preview}\n"));
                definitions.push(json!({"path":file,"hash":value["hash"],"symbol":value["symbol"]}));
            }
            Err((code @ ("CANCELLED" | "TIMEOUT"), message)) => return Err((code, message)),
            Err((code, _)) => unresolved.push(json!({"path":file,"code":code,"message":"Indexed candidate not resolved by the current local Rust parser"})),
        }
    }
    text.push_str("\nRelaciones reportadas por el índice (no verificadas en código actual):\n");
    for edge in edges.iter().take(20) {
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
    let partial = truncated || !unresolved.is_empty() || targets.is_empty() || !matching;
    let traversal = json!({"direction":direction,"maxDepth":max_depth,"nodeDepths":depths,"expandedNodes":expanded,"depthBoundaryReached":depth_boundary,
        "limits":{"edges":160,"nodes":176,"edgesPerNodeDirection":20},"semantics":"independent directed breadth-first traversals from selected exact-name targets; depths are minimum discovered hops within scanned edges per direction, not global shortest paths"});
    let graph = json!({"format":"codegraph-sqlite-readonly-v1","schemaVersion":version,"files":files,"nodes":nodes.values().collect::<Vec<_>>(),"edges":edges,"truncated":truncated,"traversal":traversal});
    Ok(
        json!({"text":preview,"focus":{"symbol":symbol,"status":if definitions.is_empty() {"unresolved"} else {"resolved"}},
        "definitions":definitions,"unresolved":unresolved,"graph":graph,"relationshipSource":"codegraph-index",
        "indexFreshness":"not_certified","synchronized":false,"absenceProven":false,
        "fileFreshness":freshness,"diagnostics":diagnostics,
        "synchronization":{"performed":false,"nativeIndexerAvailable":false,"reason":"native_multilanguage_indexer_not_migrated"},
        "notice":"Bounded indexed relationships, not execution traces; local definitions verified only for Rust. No exhaustive impact, absence or index freshness claims. Preview is not replacement text.",
        "truncated":truncated || utf16_len(&text)>max_chars,"_complete":!partial,
        "_detail":{"text":text,"graph":graph,"definitions":definitions,"unresolved":unresolved,"fileFreshness":freshness,"indexFreshness":"not_certified","synchronized":false}}),
    )
}
