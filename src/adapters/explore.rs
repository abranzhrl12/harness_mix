//! Read existing CodeGraph SQLite indexes without Node, migrations or arbitrary SQL.
use super::project::Project;
mod data_trace;
mod database;
mod graph;
mod model;
mod presentation;
mod response;
mod selection;
mod sources;
use harness_domain::{
    Failure,
    contracts::{ExploreArgs, ExploreDirection, ExploreView},
    execution::OperationContext,
};
use model::Node;
use rusqlite::Row;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

const INDEX: Failure = (
    "INDEX",
    "CodeGraph index access failed; inspect data.indexDiagnostic for the cause, path and schema",
);
const INDEX_BUSY: Failure = (
    "INDEX_BUSY",
    "CodeGraph index remained locked; wait for the other process and retry",
);
const CONTRACT: Failure = (
    "GRAPH_CONTRACT",
    "CodeGraph returned invalid or restricted graph metadata",
);
const BUDGET: Failure = (
    "GRAPH_LIMIT",
    "CodeGraph query exceeded its bounded read budget",
);

fn node(row: &Row<'_>, offset: usize) -> rusqlite::Result<Node> {
    Ok(Node {
        id: row.get(offset)?,
        name: row.get(offset + 1)?,
        kind: row.get(offset + 2)?,
        file_path: row.get(offset + 3)?,
        start_line: row.get(offset + 4)?,
    })
}

fn validate_node_fields(item: &mut Node) -> Result<(), Failure> {
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
    if item.file_path.contains([':', '\0'])
        || item
            .file_path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(CONTRACT);
    }
    Ok(())
}

fn validate_node(project: &Project, item: &mut Node) -> Result<(), Failure> {
    validate_node_fields(item)?;
    // Also reject metadata/credential paths and validate existing ancestors for
    // stale/deleted indexed files. Never echo an unchecked path from the index.
    if super::project::external_restricted(&item.file_path)
        || super::project::external_sensitive(&item.file_path)
    {
        return Err(CONTRACT);
    }
    project
        .target_optional(&item.file_path)
        .map_err(|_| CONTRACT)?;
    Ok(())
}

fn validate_neighbor_node(project: &Project, item: &mut Node) -> Result<bool, Failure> {
    validate_node_fields(item)?;
    if super::project::external_restricted(&item.file_path)
        || super::project::external_sensitive(&item.file_path)
        || project.target_optional(&item.file_path).is_err()
    {
        return Ok(false);
    }
    Ok(true)
}

fn identifier(text: &str) -> bool {
    let mut chars = text.chars();
    let first = chars.next();
    if first == Some('@') {
        let mut rest = chars;
        return rest
            .next()
            .is_some_and(|c| c == '_' || c == '$' || c.is_alphabetic())
            && rest.all(|c| c == '_' || c == '$' || c.is_alphanumeric());
    }
    first.is_some_and(|c| c == '_' || c == '$' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c == '$' || c.is_alphanumeric())
}

pub(super) fn query_symbols(args: &ExploreArgs) -> Vec<&str> {
    if let Some(symbol) = args.symbol.as_deref() {
        return vec![symbol.strip_prefix('@').unwrap_or(symbol)];
    }
    args.query
        .split_whitespace()
        .map(|s| s.strip_prefix('@').unwrap_or(s))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn razor_file_selector(args: &ExploreArgs) -> bool {
    args.query.trim().is_empty()
        && args.symbol.is_none()
        && args.symbol_id.is_none()
        && args.container.is_none()
        && args.view != Some(ExploreView::Impact)
        && args.file_path.as_deref().is_some_and(|path| {
            std::path::Path::new(path)
                .extension()
                .and_then(|extension| extension.to_str())
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
    if (report["selectedScopeApplied"] != true && report["sourceScopeApplied"] != true)
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
        "databasePath": project.root.join(project.index_rel("codegraph.db")),
        "stage": "filesystem", "expectedSchemaVersion": {"min":1,"max":11},
        "globalFreshnessCertified": false
    }));
    match explore_inner(project, args, context, &diagnostic) {
        Err((code @ ("INDEX" | "INDEX_BUSY"), message)) => Ok(json!({
            "ok":false, "explorationPerformed":false,
            "diagnostics":[{"code":code,"message":message}],
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
    let file_selector = args
        .file_path
        .as_deref()
        .map(|file| {
            // Windows callers pass native separators; the graph stores
            // POSIX-style paths. Normalize the selector before the graph query.
            let forward = file.replace('\\', "/");
            forward
                .trim_start_matches("./")
                .trim_end_matches('/')
                .to_string()
        });
    if let Some(file) = &args.file_path {
        project.target(file)?;
    }
    let impact = args.view == Some(ExploreView::Impact);
    let raw_symbol = args.symbol.as_deref().unwrap_or(args.query.trim());
    let symbol = raw_symbol.strip_prefix('@').unwrap_or(raw_symbol);
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
    let path = database::index_path(project, diagnostic)?;
    #[cfg(windows)]
    let _pins = super::edit_commit::pin_directory(project, path.parent().ok_or(INDEX)?)?;
    let started = Instant::now();
    let failed = |error: rusqlite::Error| database::failure(error, diagnostic, context, started);
    let opened = database::open(&path, context, diagnostic, started)?;
    let db = opened.connection;
    let version = opened.version;
    let has_qualified = opened.has_qualified;
    if args.container.is_some() && !has_qualified {
        return Err((
            "ARGUMENT",
            "This legacy index lacks qualified_name; select with filePath or symbolId",
        ));
    }
    diagnostic.borrow_mut()["stage"] = json!("graph_query");
    let selection = match selection::select(
        &db,
        project,
        args,
        file_selector.as_deref(),
        context,
        symbol,
        max_files,
        has_qualified,
        &failed,
    )? {
        selection::SelectionResult::NotResolved => {
            let mut result = selection::not_resolved_value();
            let archive = if let Some(path) = file_selector.as_deref() {
                super::index_store::quarantine::coverage(&db, Some(path), None)?
            } else {
                super::index_store::quarantine::Coverage::default()
            };
            super::index_store::quarantine::annotate(&mut result, &archive, true);
            db.execute_batch("ROLLBACK").map_err(failed)?;
            if let Ok(Some(trace)) = data_trace::trace(
                project,
                args,
                raw_symbol,
                &[],
                &BTreeSet::new(),
                context,
            ) {
                if let Ok(trace_json) = serde_json::to_value(&trace) {
                    result["dataTrace"] = trace_json.clone();
                    result["crossLayerTrace"] = trace_json;
                }
            }
            return Ok(result);
        }
        selection::SelectionResult::Selected(selection) => selection,
    };
    let symbol = selection.symbol.clone();
    let graph = graph::traverse(
        &db,
        project,
        context,
        diagnostic,
        started,
        &selection.targets,
        direction,
        max_depth,
        selection.truncated,
    )?;
    let mut graph = graph;
    let mut contributing_files = graph
        .nodes
        .values()
        .map(|node| node.file_path.clone())
        .collect::<BTreeSet<_>>();
    contributing_files.extend(graph.export_contributors.keys().cloned());
    let archive = super::index_store::quarantine::coverage(
        &db, None, Some(&graph.nodes.keys().cloned().collect()),
    )?;
    let mut indexed_files =
        super::index_freshness::indexed_files(&db, contributing_files, context)?;
    for (path, digest) in std::mem::take(&mut graph.export_contributors) {
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
    let loaded = sources::load(
        project,
        args,
        context,
        &selection.targets,
        &graph.nodes,
        &graph.edges,
        max_files,
        max_chars,
        summary,
        &symbol,
    )?;
    let graph_files = graph
        .nodes
        .values()
        .map(|node| node.file_path.clone())
        .collect::<BTreeSet<_>>();
    let trace_result = data_trace::trace(
        project,
        args,
        &symbol,
        &selection.targets,
        &graph_files,
        context,
    )?;
    let mut result = response::respond(
        project,
        args,
        context,
        assessment,
        response::ResponseInput {
            symbol,
            impact,
            summary,
            direction,
            max_depth,
            max_chars,
            max_files,
            version,
            selection,
            graph,
            sources: loaded,
            data_trace: trace_result,
        },
    )?;
    super::index_store::quarantine::annotate(&mut result, &archive, true);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};
    use std::path::Path;

    fn setup_db(root: &Path) -> Connection {
        std::fs::create_dir_all(root.join(".codegraph")).unwrap();
        let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE schema_versions(version INTEGER PRIMARY KEY); INSERT INTO schema_versions VALUES(11);
            CREATE TABLE nodes(id TEXT PRIMARY KEY,name TEXT,kind TEXT,file_path TEXT,start_line INTEGER);
            CREATE INDEX idx_nodes_name ON nodes(name);
            CREATE TABLE edges(id INTEGER PRIMARY KEY,source TEXT,target TEXT,kind TEXT);
            CREATE INDEX idx_edges_source ON edges(source); CREATE INDEX idx_edges_target ON edges(target);
            CREATE TABLE files(path TEXT PRIMARY KEY,content_hash TEXT,errors TEXT);",
        )
        .unwrap();
        db
    }

    #[test]
    fn skips_restricted_and_sensitive_neighbor_nodes_without_aborting() {
        let root = tempfile::tempdir().unwrap();
        let db = setup_db(root.path());
        std::fs::write(
            root.path().join("lib.rs"),
            "pub fn target() {}\npub fn caller() { target(); }\n",
        )
        .unwrap();
        std::fs::write(root.path().join("tailwind.config.ts"), "export default {};\n").unwrap();
        std::fs::write(root.path().join(".env"), "SECRET=1\n").unwrap();
        db.execute_batch(
            "INSERT INTO nodes VALUES('t','target','function','lib.rs',1);
             INSERT INTO nodes VALUES('c','caller','function','lib.rs',2);
             INSERT INTO nodes VALUES('cfg','config','variable','tailwind.config.ts',1);
             INSERT INTO nodes VALUES('env','env_var','variable','.env',1);
             INSERT INTO nodes VALUES('sec','secret_fn','function','secrets/helper.rs',1);
             INSERT INTO edges VALUES(1,'c','t','calls');
             INSERT INTO edges VALUES(2,'cfg','t','references');
             INSERT INTO edges VALUES(3,'t','env','references');
             INSERT INTO edges VALUES(4,'sec','t','calls');",
        )
        .unwrap();
        db.execute(
            "INSERT INTO files VALUES('lib.rs',?1,NULL)",
            [harness_domain::hash(
                &std::fs::read(root.path().join("lib.rs")).unwrap(),
            )],
        )
        .unwrap();

        let project = Project::new(root.path()).unwrap();
        let args: ExploreArgs = serde_json::from_value(json!({"query": "target"})).unwrap();
        let result = explore(&project, &args, &OperationContext::default()).unwrap();

        let node_ids: Vec<_> = result["graph"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|n| n["id"].as_str())
            .collect();
        assert!(node_ids.contains(&"t"));
        assert!(node_ids.contains(&"c"));
        assert!(!node_ids.contains(&"cfg"));
        assert!(!node_ids.contains(&"env"));
        assert!(!node_ids.contains(&"sec"));

        // Corrupt path traversal neighbor still aborts with GRAPH_CONTRACT.
        db.execute_batch(
            "INSERT INTO nodes VALUES('bad','hidden','function','../private-edge.rs',1);
             INSERT INTO edges VALUES(5,'bad','t','calls');",
        )
        .unwrap();
        let err = explore(&project, &args, &OperationContext::default()).unwrap_err();
        assert_eq!(err, CONTRACT);
    }

    #[test]
    fn qualifies_and_deduplicates_homonymous_targets_in_call_paths() {
        let root = tempfile::tempdir().unwrap();
        let db = setup_db(root.path());
        std::fs::create_dir_all(root.path().join("src/a")).unwrap();
        std::fs::create_dir_all(root.path().join("src/b")).unwrap();
        std::fs::write(
            root.path().join("src/a/AuthLayout.tsx"),
            "export interface AuthLayout {}\nexport function AuthLayout() {}\nexport function app() { AuthLayout(); }\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("src/b/AuthLayout.tsx"),
            "export function AuthLayout() {}\n",
        )
        .unwrap();

        for (id, name, kind, path, line) in [
            ("a_type", "AuthLayout", "interface", "src/a/AuthLayout.tsx", 1),
            ("a_fn", "AuthLayout", "function", "src/a/AuthLayout.tsx", 2),
            ("app", "app", "function", "src/a/AuthLayout.tsx", 3),
            ("b_fn1", "AuthLayout", "function", "src/b/AuthLayout.tsx", 1),
            ("b_fn2", "AuthLayout", "function", "src/b/AuthLayout.tsx", 1),
        ] {
            db.execute(
                "INSERT INTO nodes VALUES(?1,?2,?3,?4,?5)",
                params![id, name, kind, path, line],
            )
            .unwrap();
        }
        db.execute(
            "INSERT INTO edges VALUES(1,'app','a_fn','calls')",
            [],
        )
        .unwrap();
        for path in ["src/a/AuthLayout.tsx", "src/b/AuthLayout.tsx"] {
            db.execute(
                "INSERT INTO files VALUES(?1,?2,NULL)",
                params![
                    path,
                    harness_domain::hash(&std::fs::read(root.path().join(path)).unwrap())
                ],
            )
            .unwrap();
        }

        let project = Project::new(root.path()).unwrap();
        let args: ExploreArgs = serde_json::from_value(json!({"query": "AuthLayout"})).unwrap();
        let result = explore(&project, &args, &OperationContext::default()).unwrap();
        let text = result["_detail"]["text"].as_str().unwrap();

        assert!(
            text.contains("- AuthLayout (function at src/a/AuthLayout.tsx:2): 1 unique direct callers"),
            "{text}"
        );
        assert_eq!(
            text.matches("- AuthLayout (function at src/b/AuthLayout.tsx:1): 0 unique direct callers")
                .count(),
            1,
            "{text}"
        );
        assert!(
            !text.contains("AuthLayout (interface at src/a/AuthLayout.tsx:1)"),
            "{text}"
        );

        // Single-target formatting remains unqualified.
        let single_args: ExploreArgs = serde_json::from_value(json!({"query": "app"})).unwrap();
        let single_result = explore(&project, &single_args, &OperationContext::default()).unwrap();
        let single_text = single_result["_detail"]["text"].as_str().unwrap();
        assert!(
            single_text.contains("- app: 0 unique direct callers"),
            "{single_text}"
        );
    }
}
