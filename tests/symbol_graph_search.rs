//! Graph-backed symbol discovery: candidates, certification and repair.
use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{Kind, Request, SearchArgs};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::Path;

fn search(app: &NativeApplication, query: &str) -> Value {
    app.execute(Request::Search(SearchArgs {
            override_project_exclusions: None,
        only_matching: None,
        ast: None,
        regex: None,
        case_sensitive: None,
        context_lines: None,
        kind: Kind::Syntax,
        query: query.into(),
        min_lines: None,
        globs: None,
        scope: Some(".".into()),
        limit: Some(40),
        session: None,
            cursor: None,
            offset: None,
        external_scope: None,
    }))
}

/// Minimal CodeGraph-compatible index with the schema version the harness reads.
fn index(
    root: &Path,
    files: &[(&str, &str)],
    nodes: &[(&str, &str, &str, &str, u32)],
) -> Connection {
    std::fs::create_dir_all(root.join(".codegraph")).unwrap();
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE schema_versions(version INTEGER PRIMARY KEY, applied_at INTEGER, description TEXT);
         INSERT INTO schema_versions VALUES(11,0,'test');
         CREATE TABLE nodes(id TEXT PRIMARY KEY,name TEXT,kind TEXT,file_path TEXT,start_line INTEGER,
           end_line INTEGER,qualified_name TEXT,signature TEXT,docstring TEXT,language TEXT);
         CREATE TABLE files(path TEXT PRIMARY KEY,content_hash TEXT,language TEXT,size INTEGER,
           modified_at INTEGER,indexed_at INTEGER,node_count INTEGER,errors TEXT,generated INTEGER);",
    )
    .unwrap();
    for (path, source) in files {
        let bytes = std::fs::read(root.join(path)).unwrap();
        db.execute(
            "INSERT INTO files VALUES(?1,?2,'csharp',?3,0,0,1,'[]',0)",
            params![path, harness_domain::hash(&bytes), bytes.len() as i64],
        )
        .unwrap();
        let _ = source;
    }
    for (id, name, kind, path, line) in nodes {
        db.execute(
            "INSERT INTO nodes VALUES(?1,?2,?3,?4,?5,?5,?2,?6,'', 'csharp')",
            params![id, name, kind, path, line, format!("{name}()")],
        )
        .unwrap();
    }
    db
}

#[test]
fn razor_selectors_work_live_and_repair_stale_graph_declarations() {
    let root = tempfile::tempdir().unwrap();
    let source = "@model Demo.Model\n@section Scripts { <script>fake();</script> }\n@helper Label() { <b>Label</b> }\n";
    std::fs::write(root.path().join("Index.cshtml"), source).unwrap();
    let app = adapters::application(root.path()).unwrap();
    for selector in [
        "razor_model:Demo.Model",
        "razor_section:Scripts",
        "razor_helper:Label",
    ] {
        let result = search(&app, selector);
        assert_eq!(result["state"], "ok", "{result}");
        assert_eq!(
            result["data"]["matches"].as_array().unwrap().len(),
            1,
            "{result}"
        );
    }
    let db = index(
        root.path(),
        &[("Index.cshtml", source)],
        &[("section", "Scripts", "module", "Index.cshtml", 2)],
    );
    db.execute("UPDATE nodes SET language='razor';", [])
        .unwrap();
    let fresh = search(&app, "razor_section:Scripts");
    assert_eq!(fresh["data"]["discovery"], "index", "{fresh}");
    std::fs::write(root.path().join("Index.cshtml"), "@section Removed {}\n").unwrap();
    let stale = search(&app, "razor_section:Scripts");
    assert!(
        stale["data"]["matches"].as_array().unwrap().is_empty(),
        "{stale}"
    );
}

#[test]
fn graph_search_respects_project_ignore() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("ignored")).unwrap();
    let source = "@section Scripts { }\n";
    std::fs::write(root.path().join("ignored/Index.cshtml"), source).unwrap();
    std::fs::write(root.path().join(".harnessignore"), "ignored/\n").unwrap();
    let db = index(root.path(), &[("ignored/Index.cshtml", source)], &[("section", "Scripts", "module", "ignored/Index.cshtml", 1)]);
    db.execute("UPDATE nodes SET language='razor';", []).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = search(&app, "razor_section:Scripts");
    assert_eq!(result["state"], "ok", "{result}");
    assert!(result["data"]["matches"].as_array().unwrap().is_empty(), "{result}");
}

#[test]
fn without_index_the_live_path_answers_and_declares_itself() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("lib.rs"), "pub fn real() {}\n").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = search(&app, "function_item:real");
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["discovery"], "live", "{result}");
    assert_eq!(result["data"]["language"], "rust");
    let matches = result["data"]["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["name"], "real");
}

#[test]
fn a_fresh_index_publishes_certified_symbols_without_reading_the_tree() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    std::fs::write(
        root.path().join("src/Service.cs"),
        "public class Service { public void Procesar() {} }\n",
    )
    .unwrap();
    index(
        root.path(),
        &[("src/Service.cs", "")],
        &[("n1", "Procesar", "method", "src/Service.cs", 1)],
    );
    let app = adapters::application(root.path()).unwrap();
    let result = search(&app, "function_item:Procesar");
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["discovery"], "index", "{result}");
    assert_eq!(result["data"]["languages"], json!(["csharp"]));
    assert_eq!(result["data"]["certifiedFiles"], 1);
    assert_eq!(result["data"]["repairedFiles"], 0);
    assert_eq!(result["data"]["absenceProven"], false);
    let matches = result["data"]["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1, "{result}");
    assert_eq!(matches[0]["name"], "Procesar");
    assert_eq!(matches[0]["file"], "src/Service.cs");
    assert_eq!(matches[0]["freshness"], "fresh");
    assert_eq!(
        matches[0]["hash"],
        harness_domain::hash(&std::fs::read(root.path().join("src/Service.cs")).unwrap())
    );
}

#[test]
fn a_changed_file_is_repaired_live_instead_of_publishing_the_stale_symbol() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    std::fs::write(
        root.path().join("src/Service.cs"),
        "public class Service { public void Procesar() {} }\n",
    )
    .unwrap();
    index(
        root.path(),
        &[("src/Service.cs", "")],
        &[("n1", "Procesar", "method", "src/Service.cs", 1)],
    );
    // The source changes after the index was written.
    std::fs::write(
        root.path().join("src/Service.cs"),
        "public class Service { public void ProcesarAsync() {} }\n",
    )
    .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = search(&app, "function_item:Procesar");
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["discovery"], "index", "{result}");
    assert_eq!(result["data"]["repairedFiles"], 1, "{result}");
    let matches = result["data"]["matches"].as_array().unwrap();
    // The stale index still knows "Procesar"; the repaired live parse does not.
    assert!(matches.is_empty(), "{result}");
}

#[test]
fn a_deleted_file_is_dropped_and_reported_instead_of_being_published() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    std::fs::write(
        root.path().join("src/Service.cs"),
        "public class Service { public void Procesar() {} }\n",
    )
    .unwrap();
    index(
        root.path(),
        &[("src/Service.cs", "")],
        &[("n1", "Procesar", "method", "src/Service.cs", 1)],
    );
    std::fs::remove_file(root.path().join("src/Service.cs")).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = search(&app, "function_item:Procesar");
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["missingFiles"], 1, "{result}");
    let matches = result["data"]["matches"].as_array().unwrap();
    assert!(matches.is_empty(), "{result}");
}

#[test]
fn an_index_that_does_not_cover_the_scope_leaves_the_live_path_in_charge() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    std::fs::create_dir_all(root.path().join("other")).unwrap();
    std::fs::write(root.path().join("src/Service.cs"), "class Service {}\n").unwrap();
    std::fs::write(root.path().join("other/lib.rs"), "pub fn real() {}\n").unwrap();
    index(
        root.path(),
        &[("src/Service.cs", "")],
        &[("n1", "Procesar", "method", "src/Service.cs", 1)],
    );
    let app = adapters::application(root.path()).unwrap();
    let scoped = app.execute(Request::Search(SearchArgs {
            override_project_exclusions: None,
        only_matching: None,
        ast: None,
        regex: None,
        case_sensitive: None,
        context_lines: None,
        kind: Kind::Syntax,
        query: "function_item:real".into(),
        min_lines: None,
        globs: None,
        scope: Some("other".into()),
        limit: Some(40),
        session: None,
            cursor: None,
            offset: None,
        external_scope: None,
    }));
    // The index does not cover "other", so the live path answers.
    assert_eq!(scoped["data"]["discovery"], "live", "{scoped}");
    let matches = scoped["data"]["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1, "{scoped}");
}

/// End-to-end check against the real indexer: schema, hashes and node kinds
/// are the ones the harness publishes, not a hand-written fixture.
#[cfg(windows)]
#[test]
fn a_real_index_publishes_certified_symbols() {
    use agent_harness::adapters::index_store::{initialize, sync_workspace};
    use harness_domain::execution::OperationContext;

    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    initialize(root.path(), &context).unwrap();
    std::fs::write(
        root.path().join("Service.cs"),
        "public class Service { public void Procesar() {} }\n",
    )
    .unwrap();
    std::fs::write(root.path().join("lib.rs"), "pub fn helper() {}\n").unwrap();
    sync_workspace(root.path(), &context).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = search(&app, "function_item:Procesar");
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["discovery"], "index", "{result}");
    let matches = result["data"]["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1, "{result}");
    assert_eq!(matches[0]["name"], "Procesar");
    assert_eq!(matches[0]["file"], "Service.cs");
    assert_eq!(matches[0]["freshness"], "fresh");
}
