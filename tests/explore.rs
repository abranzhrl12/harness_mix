#[path = "support/codegraph.rs"]
mod graph_fixture;

use agent_harness::adapters;
use harness_domain::{
    contracts::{ExploreArgs, ExploreDirection, Request},
    execution::OperationContext,
};
use rusqlite::params;
use serde_json::{Value, json};


#[test]
#[ignore = "opt-in retrieval comparison against an existing index; no synchronization"]
fn compare_context_on_existing_index() {
    let root = std::path::PathBuf::from(std::env::var_os("HARNESS_CODEGRAPH_ROOT").expect("HARNESS_CODEGRAPH_ROOT"));
    let app = adapters::application(&root).unwrap();
    for query in ["sync_workspace_all", "synchronized", "sync_workspace_all synchronized"] {
        let started = std::time::Instant::now();
        let result = explore(&app, query);
        let text = result["data"]["text"].as_str().unwrap_or("");
        println!("{}", json!({
            "query": query, "elapsedMs": started.elapsed().as_millis(),
            "state": result["state"], "diagnostics": result["diagnostics"],
            "textChars": text.chars().count(),
            "definitions": result["data"]["definitions"],
            "nodes": result["data"]["graph"]["nodes"].as_array().map(Vec::len),
            "edges": result["data"]["graph"]["edges"].as_array().map(Vec::len),
            "freshness": result["data"]["fileFreshness"]["status"],
            "preview": text.chars().take(1800).collect::<String>()
        }));
    }
}


#[test]
fn context_includes_direct_callees_and_callers_and_accepts_multiple_names() {
    let root = tempfile::tempdir().unwrap();
    drop(graph_fixture::fixture(root.path()));
    let app = adapters::application(root.path()).unwrap();
    let result = explore(&app, "target");
    assert_eq!(result["state"], "ok", "{result}");
    let definitions = result["data"]["definitions"].as_array().unwrap();
    assert!(definitions.iter().any(|d| d["symbol"]["name"] == "callee" && d["role"] == "related"));
    assert!(definitions.iter().any(|d| d["symbol"]["name"] == "caller" && d["role"] == "related"));
    let text = result["data"]["text"].as_str().unwrap();
    assert!(text.contains("unique direct callers"));
    assert!(text.contains("pub fn callee()"));
    let multi = explore(&app, "target callee target");
    assert_eq!(multi["state"], "ok", "{multi}");
    assert_eq!(multi["data"]["contextSelection"]["requestedSymbols"], json!(["callee", "target"]));
    assert_eq!(multi["data"]["definitions"].as_array().unwrap().iter().filter(|d| d["role"] == "target").count(), 2);
    let mut summary = args("target");
    summary.view = Some(harness_domain::contracts::ExploreView::Summary);
    let compact = app.execute(Request::Explore(summary));
    assert_eq!(compact["data"]["contextSelection"]["relatedReturned"], 0);
    assert!(!compact["data"]["text"].as_str().unwrap().contains("pub fn callee()"));
    assert_eq!(explore(&app, "a b c d e")["state"], "error");
}


#[test]
fn context_caps_related_bodies_and_deduplicates_only_presentation() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    let mut source = std::fs::read_to_string(root.path().join("lib.rs")).unwrap();
    for n in 0..6 {
        let name = format!("neighbor{n}");
        source.push_str(&format!("pub fn {name}() {{ target(); }}\n"));
        db.execute("INSERT INTO nodes VALUES(?1,?1,'function','lib.rs',?2)", params![name, n + 4]).unwrap();
        db.execute("INSERT INTO edges(source,target,kind) VALUES(?1,'t','calls')", [&name]).unwrap();
    }
    db.execute("INSERT INTO edges(source,target,kind) VALUES('c','t','calls')", []).unwrap();
    std::fs::write(root.path().join("lib.rs"), &source).unwrap();
    db.execute("UPDATE files SET content_hash=?1 WHERE path='lib.rs'", [harness_domain::hash(source.as_bytes())]).unwrap();
    drop(db);
    let app = adapters::application(root.path()).unwrap();
    let result = explore(&app, "target");
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["contextSelection"]["relatedReturned"], 4);
    assert_eq!(result["data"]["contextSelection"]["relatedOmitted"], 4);
    let text = result["data"]["text"].as_str().unwrap();
    assert_eq!(text.matches("caller (lib.rs:2) → target").count(), 1);
    assert!(text.contains("7 unique direct callers"));
    assert!(text.contains("related symbols omitted"));
    assert_eq!(result["data"]["graph"]["edges"].as_array().unwrap().iter().filter(|e| e["source"] == "c" && e["kind"] == "calls").count(), 2);
}

fn args(symbol: &str) -> ExploreArgs {
    serde_json::from_value(json!({"query":symbol})).unwrap()
}
fn explore(app: &adapters::NativeApplication, symbol: &str) -> Value {
    app.execute(Request::Explore(args(symbol)))
}

#[test]
fn contributing_files_are_checked_and_strict_mode_requires_matching_content() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    let app = adapters::application(root.path()).unwrap();
    let strict = || {
        let mut request = args("target");
        request.require_matching_files = Some(true);
        app.execute(Request::Explore(request))
    };
    let initial = strict();
    assert_eq!(initial["state"], "ok", "{initial}");
    assert_eq!(
        initial["data"]["fileFreshness"]["status"],
        "matches_observed_files"
    );
    assert_eq!(
        initial["data"]["fileFreshness"]["certifiesIndexFreshness"],
        false
    );
    assert_eq!(initial["data"]["indexFreshness"], "not_certified");
    std::fs::write(
        root.path().join("lib.rs"),
        "pub fn target() { /* edited */ }\n",
    )
    .unwrap();
    let stale = explore(&app, "target");
    assert_eq!(stale["state"], "ok", "{stale}");
    assert_eq!(stale["data"]["fileFreshness"]["status"], "stale");
    assert_eq!(stale["detail"]["complete"], false);
    assert_eq!(stale["diagnostics"][0]["code"], "INDEX_STALE");
    let blocked = strict();
    assert_eq!(blocked["diagnostics"][0]["code"], "INDEX_STALE");
    assert!(blocked["data"].as_object().unwrap().is_empty());
    // Simulate the provider recording an updated hash only in this isolated fixture.
    // The native adapter never rewrites provider metadata to manufacture a match.
    db.execute(
        "UPDATE files SET content_hash=?1 WHERE path='lib.rs'",
        [harness_domain::hash(
            &std::fs::read(root.path().join("lib.rs")).unwrap(),
        )],
    )
    .unwrap();
    assert_eq!(
        strict()["data"]["fileFreshness"]["status"],
        "matches_observed_files"
    );
    std::fs::remove_file(root.path().join("lib.rs")).unwrap();
    let deleted = explore(&app, "target");
    assert_eq!(
        deleted["data"]["fileFreshness"]["files"][0]["reason"],
        "source_missing"
    );
    assert_eq!(strict()["diagnostics"][0]["code"], "INDEX_STALE");
}

#[test]
fn freshness_covers_related_files_and_never_certifies_unindexed_changes() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    std::fs::write(root.path().join("related.js"), "function helper() {}\n").unwrap();
    db.execute_batch(
        "INSERT INTO nodes VALUES('related','helper','function','related.js',1);
        INSERT INTO edges VALUES(4,'d','related','calls');",
    )
    .unwrap();
    db.execute(
        "INSERT INTO files VALUES('related.js',?1,NULL)",
        [harness_domain::hash(b"function helper() {}\n")],
    )
    .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let mut request = args("target");
    request.direction = Some(ExploreDirection::Outbound);
    request.max_depth = Some(2);
    let initial = app.execute(Request::Explore(request));
    assert_eq!(initial["data"]["fileFreshness"]["matchingFiles"], 2);
    std::fs::write(root.path().join("related.js"), "function changed() {}\n").unwrap();
    let mut request = args("target");
    request.direction = Some(ExploreDirection::Outbound);
    request.max_depth = Some(2);
    request.require_matching_files = Some(true);
    let blocked = app.execute(Request::Explore(request));
    assert_eq!(blocked["diagnostics"][0]["code"], "INDEX_STALE");
    // An unvisited/unindexed file must not turn partial checks into a full-index claim.
    std::fs::write(
        root.path().join("unindexed.rs"),
        "pub fn another_caller() { target(); }\n",
    )
    .unwrap();
    let shallow = explore(&app, "target");
    assert_eq!(
        shallow["data"]["fileFreshness"]["status"],
        "matches_observed_files"
    );
    assert_eq!(
        shallow["data"]["fileFreshness"]["unindexedFilesChecked"],
        false
    );
    assert_eq!(
        shallow["data"]["fileFreshness"]["wholeWorkspaceChecked"],
        false
    );
    assert_eq!(shallow["data"]["synchronization"]["performed"], false);
}

#[test]
fn missing_invalid_or_failed_extraction_metadata_and_limits_are_unknown() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    let app = adapters::application(root.path()).unwrap();
    let strict = || {
        let mut request = args("target");
        request.require_matching_files = Some(true);
        app.execute(Request::Explore(request))
    };
    db.execute("UPDATE files SET errors='[\"private parser failure\"]'", [])
        .unwrap();
    let failed = explore(&app, "target");
    assert_eq!(failed["data"]["fileFreshness"]["status"], "unknown");
    assert_eq!(strict()["diagnostics"][0]["code"], "INDEX_UNCHECKED");
    assert!(!failed.to_string().contains("private parser failure"));
    db.execute("UPDATE files SET errors=NULL,content_hash='invalid'", [])
        .unwrap();
    assert_eq!(
        explore(&app, "target")["data"]["fileFreshness"]["files"][0]["reason"],
        "invalid_indexed_hash"
    );
    db.execute("DELETE FROM files", []).unwrap();
    assert_eq!(strict()["diagnostics"][0]["code"], "INDEX_UNCHECKED");
    db.execute_batch(
        "DROP TABLE files; CREATE VIEW files AS SELECT 'private metadata' AS content_hash;",
    )
    .unwrap();
    let view = explore(&app, "target");
    assert_eq!(view["data"]["fileFreshness"]["status"], "unknown");
    assert!(!view.to_string().contains("private metadata"));
    db.execute_batch(
        "DROP VIEW files; CREATE TABLE files(path TEXT PRIMARY KEY,content_hash TEXT,errors TEXT);",
    )
    .unwrap();
    std::fs::write(root.path().join("lib.rs"), vec![b'x'; 2 * 1024 * 1024 + 1]).unwrap();
    db.execute(
        "INSERT INTO files VALUES('lib.rs',?1,NULL)",
        [harness_domain::hash(b"original")],
    )
    .unwrap();
    assert_eq!(
        explore(&app, "target")["data"]["fileFreshness"]["files"][0]["reason"],
        "source_size_budget"
    );
    db.execute(
        "UPDATE files SET content_hash=?1",
        [harness_domain::hash(b"codegraph:oversize:2097153")],
    )
    .unwrap();
    assert_eq!(
        explore(&app, "target")["data"]["fileFreshness"]["files"][0]["reason"],
        "oversize_index_marker"
    );
    assert_eq!(strict()["diagnostics"][0]["code"], "INDEX_UNCHECKED");
}

#[test]
fn multihop_keeps_directions_separate_and_reports_minimum_depths() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    for name in ["a", "b", "sibling", "other_caller"] {
        db.execute(
            "INSERT INTO nodes VALUES(?1,?1,'function','lib.rs',1)",
            [name],
        )
        .unwrap();
    }
    db.execute_batch("INSERT INTO edges VALUES(4,'a','c','calls'); INSERT INTO edges VALUES(5,'d','b','calls');
        INSERT INTO edges VALUES(6,'c','sibling','calls'); INSERT INTO edges VALUES(7,'other_caller','d','calls');").unwrap();
    let app = adapters::application(root.path()).unwrap();
    for (direction, expected, absent) in [
        (
            ExploreDirection::Inbound,
            vec!["t", "c", "a"],
            vec!["d", "b", "sibling", "other_caller"],
        ),
        (
            ExploreDirection::Outbound,
            vec!["t", "d", "b"],
            vec!["c", "a", "sibling", "other_caller"],
        ),
        (
            ExploreDirection::Both,
            vec!["t", "c", "a", "d", "b"],
            vec!["sibling", "other_caller"],
        ),
    ] {
        let mut request = args("target");
        request.direction = Some(direction);
        request.max_depth = Some(2);
        let result = app.execute(Request::Explore(request));
        assert_eq!(result["state"], "ok", "{result}");
        let graph = &result["data"]["graph"];
        let ids: Vec<_> = graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), expected.len());
        for id in expected {
            assert!(ids.contains(&id), "{result}");
        }
        for id in absent {
            assert!(!ids.contains(&id), "{result}");
        }
        assert_eq!(graph["traversal"]["depthBoundaryReached"], true);
        if !matches!(direction, ExploreDirection::Outbound) {
            assert_eq!(graph["traversal"]["nodeDepths"]["a"]["inbound"], 2);
        }
        if !matches!(direction, ExploreDirection::Inbound) {
            assert_eq!(graph["traversal"]["nodeDepths"]["b"]["outbound"], 2);
        }
    }
    let default = explore(&app, "target");
    assert_eq!(default["data"]["graph"]["traversal"]["maxDepth"], 1);
    assert_eq!(
        default["data"]["graph"]["edges"].as_array().unwrap().len(),
        2
    );
    for depth in [0, 5] {
        let mut request = args("target");
        request.max_depth = Some(depth);
        assert_eq!(
            app.execute(Request::Explore(request))["diagnostics"][0]["code"],
            "ARGUMENT"
        );
    }
    assert!(
        serde_json::from_value::<ExploreArgs>(json!({"query":"target","direction":"sideways"}))
            .is_err()
    );
}

#[test]
fn cycles_self_loops_and_diamonds_do_not_repeat_expansion() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    for id in ["a", "z"] {
        db.execute(
            "INSERT INTO nodes VALUES(?1,?1,'function','lib.rs',1)",
            [id],
        )
        .unwrap();
    }
    db.execute_batch("INSERT INTO edges VALUES(4,'d','t','calls'); INSERT INTO edges VALUES(5,'d','d','calls');
        INSERT INTO edges VALUES(6,'t','a','calls'); INSERT INTO edges VALUES(7,'a','z','calls'); INSERT INTO edges VALUES(8,'d','z','calls');").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let mut request = args("target");
    request.direction = Some(ExploreDirection::Outbound);
    request.max_depth = Some(4);
    let result = app.execute(Request::Explore(request));
    assert_eq!(result["state"], "ok", "{result}");
    let graph = &result["data"]["graph"];
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 4);
    assert_eq!(graph["edges"].as_array().unwrap().len(), 6);
    assert_eq!(graph["traversal"]["expandedNodes"], 4);
    assert_eq!(graph["traversal"]["nodeDepths"]["t"]["outbound"], 0);
    assert_eq!(graph["traversal"]["nodeDepths"]["z"]["outbound"], 2);
    assert_eq!(graph["traversal"]["depthBoundaryReached"], false);
    assert_eq!(graph["truncated"], false);
}

#[test]
fn deeper_paths_are_validated_and_global_edge_budget_is_enforced() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    db.execute_batch("INSERT INTO nodes VALUES('private','hidden','function','../private-edge.rs',1); INSERT INTO edges VALUES(4,'d','private','calls');").unwrap();
    let app = adapters::application(root.path()).unwrap();
    assert_eq!(explore(&app, "target")["state"], "ok");
    let mut request = args("target");
    request.max_depth = Some(2);
    let bad = app.execute(Request::Explore(request));
    assert_eq!(bad["diagnostics"][0]["code"], "GRAPH_CONTRACT");
    assert!(!bad.to_string().contains("private-edge"));
    db.execute_batch("DELETE FROM edges WHERE id=4; DELETE FROM nodes WHERE id='private';")
        .unwrap();
    let mut edge_id = 10;
    for branch in 0..19 {
        let parent = format!("branch{branch}");
        db.execute(
            "INSERT INTO nodes VALUES(?1,?1,'function','lib.rs',1)",
            [&parent],
        )
        .unwrap();
        db.execute(
            "INSERT INTO edges VALUES(?1,'t',?2,'calls')",
            params![edge_id, parent],
        )
        .unwrap();
        edge_id += 1;
        for leaf in 0..20 {
            let child = format!("leaf{branch}_{leaf}");
            db.execute(
                "INSERT INTO nodes VALUES(?1,?1,'function','lib.rs',1)",
                [&child],
            )
            .unwrap();
            db.execute(
                "INSERT INTO edges VALUES(?1,?2,?3,'calls')",
                params![edge_id, parent, child],
            )
            .unwrap();
            edge_id += 1;
        }
    }
    let mut request = args("target");
    request.max_depth = Some(2);
    request.direction = Some(ExploreDirection::Outbound);
    let result = app.execute(Request::Explore(request));
    assert_eq!(result["state"], "ok", "{result}");
    let graph = &result["data"]["graph"];
    assert_eq!(graph["edges"].as_array().unwrap().len(), 160);
    assert!(graph["nodes"].as_array().unwrap().len() <= 176);
    assert_eq!(graph["truncated"], true);
    assert_eq!(graph["traversal"]["depthBoundaryReached"], true);
    assert_eq!(result["detail"]["complete"], false);
}

#[test]
fn exact_symbol_relations_local_definitions_and_database_remain_read_only() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    drop(db);
    let path = root.path().join(".codegraph/codegraph.db");
    let before = std::fs::read(&path).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = explore(&app, "target");
    assert_eq!(result["state"], "ok", "{result}");
    let markdown = result["data"]["text"].as_str().unwrap();
    assert!(markdown.starts_with("**Exploration: target**"));
    assert!(markdown.contains("**Source Code**"));
    assert!(markdown.contains("### lib.rs:"));
    assert!(markdown.contains("\tpub fn target()"));
    assert!(markdown.contains("**Call paths / blast radius"));
    assert_eq!(result["data"]["focus"]["status"], "resolved");
    assert_eq!(
        result["data"]["graph"]["edges"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        result["data"]["definitions"][0]["hash"],
        harness_domain::hash(&std::fs::read(root.path().join("lib.rs")).unwrap())
    );
    assert!(
        result["data"]["text"]
            .as_str()
            .unwrap()
            .contains("pub fn target()")
    );
    assert_eq!(result["data"]["indexFreshness"], "not_certified");
    assert_eq!(result["data"]["synchronized"], false);
    assert_eq!(result["data"]["absenceProven"], false);
    assert_eq!(result["detail"]["complete"], true);
    assert_eq!(std::fs::read(path).unwrap(), before);
    // Local definitions are re-read, not replaced with stale indexed bodies.
    std::fs::write(root.path().join("lib.rs"), "pub fn renamed() {}\n").unwrap();
    let stale = explore(&app, "target");
    assert_eq!(stale["data"]["focus"]["status"], "unresolved");
    let unresolved = stale["data"]["unresolved"].as_array().unwrap();
    assert_eq!(unresolved.len(), 3);
    assert_eq!(unresolved.iter().filter(|d| d["role"] == "target" && d["symbol"] == "target").count(), 1);
    assert_eq!(stale["detail"]["complete"], false);
    let absent = explore(&app, "nonexistent");
    assert_eq!(absent["data"]["absenceProven"], false);
    assert_eq!(absent["detail"]["complete"], false);
}

#[test]
fn query_and_index_validation_never_create_an_index_or_expose_raw_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    assert_eq!(
        explore(&app, "who calls target?")["diagnostics"][0]["code"],
        "ARGUMENT"
    );
    assert_eq!(explore(&app, "target")["diagnostics"][0]["code"], "INDEX");
    assert!(!root.path().join(".codegraph").exists());
    let db = graph_fixture::fixture(root.path());
    db.execute("UPDATE schema_versions SET version=999", [])
        .unwrap();
    assert_eq!(explore(&app, "target")["diagnostics"][0]["code"], "INDEX");
    db.execute("UPDATE schema_versions SET version=11", [])
        .unwrap();
    for bad in [
        "../private-source.rs",
        ".env",
        ".git/config",
        "C:/private-source.rs",
    ] {
        db.execute("UPDATE nodes SET file_path=?1 WHERE id='t'", [bad])
            .unwrap();
        let result = explore(&app, "target");
        assert_eq!(
            result["diagnostics"][0]["code"], "GRAPH_CONTRACT",
            "{result}"
        );
        assert!(!result.to_string().contains("private-source"));
    }
    db.execute("UPDATE nodes SET file_path='lib.rs' WHERE id='t'", [])
        .unwrap();
    db.execute(
        "UPDATE nodes SET file_path='../private-source.rs' WHERE id='c'",
        [],
    )
    .unwrap();
    assert_eq!(
        explore(&app, "target")["diagnostics"][0]["code"],
        "GRAPH_CONTRACT"
    );
    db.execute_batch("DROP TABLE nodes; CREATE VIEW nodes AS SELECT 'private-source' AS name;")
        .unwrap();
    assert_eq!(explore(&app, "target")["diagnostics"][0]["code"], "INDEX");
}

#[test]
fn file_relation_and_text_limits_report_partial_results() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    for index in 0..5 {
        let file = format!("extra{index}.rs");
        std::fs::write(root.path().join(&file), "pub fn target() {}\n").unwrap();
        db.execute(
            "INSERT INTO nodes VALUES(?1,'target','function',?2,1)",
            params![format!("extra{index}"), file],
        )
        .unwrap();
    }
    for index in 4..30 {
        db.execute("INSERT INTO edges VALUES(?1,'c','t','calls')", [index])
            .unwrap();
    }
    let app = adapters::application(root.path()).unwrap();
    let mut request = args("target");
    request.max_files = Some(4);
    request.max_chars = Some(1000);
    let result = app.execute(Request::Explore(request));
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(
        result["data"]["graph"]["files"].as_array().unwrap().len(),
        4
    );
    assert_eq!(result["data"]["graph"]["truncated"], true);
    assert_eq!(result["detail"]["complete"], false);
    let mut request = args("target");
    request.max_files = Some(0);
    assert_eq!(
        app.execute(Request::Explore(request))["diagnostics"][0]["code"],
        "ARGUMENT"
    );
    // Isolate the per-direction edge bound from the file bound.
    db.execute("DELETE FROM nodes WHERE id LIKE 'extra%'", [])
        .unwrap();
    let result = explore(&app, "target");
    assert_eq!(
        result["data"]["graph"]["edges"].as_array().unwrap().len(),
        21
    );
    assert_eq!(result["data"]["graph"]["truncated"], true);
    std::fs::write(
        root.path().join("lib.rs"),
        format!("pub fn target() {{ /* {} */ }}\n", "🙂".repeat(2000)),
    )
    .unwrap();
    let mut request = args("target");
    request.max_chars = Some(1000);
    let preview = app.execute(Request::Explore(request));
    assert!(harness_domain::results::utf16_len(preview["data"]["text"].as_str().unwrap()) <= 1000);
    assert_eq!(preview["omitted"]["truncated"], true);
}

#[test]
fn graph_metadata_through_directory_links_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    drop(graph_fixture::fixture(outside.path()));
    let link = root.path().join(".codegraph");
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path().join(".codegraph"))
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path().join(".codegraph"), &link).unwrap();
    #[cfg(any(windows, unix))]
    {
        let app = adapters::application(root.path()).unwrap();
        assert_eq!(explore(&app, "target")["diagnostics"][0]["code"], "INDEX");
        #[cfg(windows)]
        std::fs::remove_dir(link).unwrap();
        #[cfg(unix)]
        std::fs::remove_file(link).unwrap();
    }
}

#[test]
fn cancellation_unknown_fields_and_unsupported_candidates_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(root.path());
    let app = adapters::application(root.path()).unwrap();
    let context = OperationContext::default();
    context.cancel();
    let result = app.execute_with_context(Request::Explore(args("target")), context);
    assert_eq!(result["state"], "cancelled");
    assert!(
        serde_json::from_value::<ExploreArgs>(json!({"query":"target","sql":"DROP TABLE nodes"}))
            .is_err()
    );
    std::fs::write(root.path().join("source.py"), "def target():\n    pass\n").unwrap();
    db.execute("UPDATE nodes SET file_path='source.py' WHERE id='t'", [])
        .unwrap();
    let result = explore(&app, "target");
    assert_eq!(
        result["data"]["unresolved"][0]["code"],
        "PARSER_UNAVAILABLE"
    );
    assert_eq!(result["data"]["focus"]["status"], "unresolved");
    assert_eq!(result["detail"]["complete"], false);
    let mut explicit = args("who calls this function?");
    explicit.symbol = Some("target".into());
    assert_eq!(app.execute(Request::Explore(explicit))["state"], "ok");
}

#[test]
#[ignore = "requires HARNESS_CODEGRAPH_ROOT pointing to an existing CodeGraph index; read-only"]
fn real_codegraph_index_can_be_read_without_running_node() {
    let root = std::path::PathBuf::from(
        std::env::var_os("HARNESS_CODEGRAPH_ROOT").expect("HARNESS_CODEGRAPH_ROOT"),
    );
    let db = rusqlite::Connection::open_with_flags(
        root.join(".codegraph/codegraph.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let symbol: String = db.query_row("SELECT name FROM nodes WHERE kind IN ('function','method') AND name GLOB '[A-Za-z_]*' AND name NOT GLOB '*[^A-Za-z0-9_]*' AND length(name) BETWEEN 4 AND 100 AND file_path LIKE '%.cs' ORDER BY name LIMIT 1", [], |r| r.get(0)).unwrap();
    let app = adapters::application(&root).unwrap();
    let result = explore(&app, &symbol);
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["graph"]["schemaVersion"], 11);
    assert!(
        !result["data"]["graph"]["nodes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(result["data"]["indexFreshness"], "not_certified");
    let mut request = args(&symbol);
    request.max_depth = Some(2);
    let deeper = app.execute(Request::Explore(request));
    assert_eq!(deeper["state"], "ok", "{deeper}");
    assert_eq!(deeper["data"]["graph"]["traversal"]["maxDepth"], 2);
    assert_eq!(deeper["data"]["indexFreshness"], "not_certified");
}
