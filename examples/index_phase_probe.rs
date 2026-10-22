//! Run with HARNESS_INDEX_PROFILE=1. Reads a project and measures an isolated copy.
//! cargo run --release --example index_phase_probe -- ROOT SAMPLE_COUNT
use agent_harness::adapters::{index_extract, index_store};
use harness_domain::execution::OperationContext;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, time::Instant};

fn graph_hash(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let db = Connection::open_with_flags(
        root.join(".codegraph/codegraph.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut bytes = Vec::new();
    for query in [
        "SELECT json_array(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,docstring,signature,visibility,is_exported,is_async,is_static,is_abstract,decorators,type_parameters,return_type) FROM nodes ORDER BY id",
        "SELECT json_array(source,target,kind,line,col,metadata,provenance) FROM edges ORDER BY source,target,kind,line,col,metadata,provenance",
        "SELECT json_array(from_node_id,reference_name,reference_kind,line,col,candidates,file_path,language,status,name_tail) FROM unresolved_refs ORDER BY from_node_id,reference_name,reference_kind,line,col,candidates,file_path,language,status,name_tail",
    ] {
        let mut statement = db.prepare(query)?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            bytes.extend_from_slice(row?.as_bytes());
            bytes.push(b'\n');
            if bytes.len() > 64 * 1024 * 1024 {
                return Err("Graph fingerprint exceeds 64 MiB budget".into());
            }
        }
        bytes.push(0);
    }
    Ok(harness_domain::hash(&bytes))
}

fn graph_statistics(root: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    let db = Connection::open_with_flags(root.join(".codegraph/codegraph.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut result = serde_json::Map::new();
    for (label, query) in [
        ("nodeKinds", "SELECT kind,count(*) FROM nodes GROUP BY kind"),
        ("referenceKinds", "SELECT reference_kind,count(*) FROM unresolved_refs GROUP BY reference_kind"),
        ("importers", "SELECT file_path,count(*) FROM nodes WHERE kind='import' GROUP BY file_path"),
        ("referenceLanguages", "SELECT language,count(*) FROM unresolved_refs GROUP BY language")
    ] {
        let mut statement = db.prepare(query)?;
        let rows = statement.query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
        result.insert(label.into(),json!(rows));
    }
    Ok(Value::Object(result))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().collect();
    let source =
        Path::new(arguments.get(1).ok_or("Expected source project root")?).canonicalize()?;
    let count = arguments
        .get(2)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(40)
        .clamp(1, 200);
    if std::env::var("HARNESS_INDEX_PROFILE").as_deref() != Ok("1") {
        return Err("Set HARNESS_INDEX_PROFILE=1 to enable phase counters".into());
    }
    let temporary = tempfile::tempdir()?;
    let batch = arguments.iter().any(|value| value == "--batch");
    let root = temporary.path();
    let mut files = Vec::new();
    let mut copied_bytes = 0u64;
    let mut walker = ignore::WalkBuilder::new(&source);
    walker
        .hidden(false)
        .follow_links(false)
        .parents(false)
        .require_git(false)
        .filter_entry(|entry| {
            entry.depth() == 0
                || !matches!(
                    entry.file_name().to_str(),
                    Some(
                        ".git"
                            | ".codegraph"
                            | ".agents"
                            | "node_modules"
                            | "target"
                            | "bin"
                            | "obj"
                    )
                )
        });
    for entry in walker.build() {
        let entry = entry?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(&source)?
            .to_string_lossy()
            .replace('\\', "/");
        if index_extract::language(&relative).is_none() || entry.metadata()?.len() > 1024 * 1024 {
            continue;
        }
        if copied_bytes + entry.metadata()?.len() > 64 * 1024 * 1024 {
            return Err("Source copy exceeds 64 MiB budget".into());
        }
        let target = root.join(&relative);
        std::fs::create_dir_all(target.parent().ok_or("Missing parent")?)?;
        copied_bytes += std::fs::copy(entry.path(), target)?;
        files.push(relative);
    }
    files.sort();
    // Preserve language/configuration selection without copying service credentials.
    for name in ["codegraph.json", "tsconfig.json"] {
        let candidate = source.join(name);
        if candidate.is_file() {
            std::fs::copy(candidate, root.join(name))?;
        }
    }
    let context = OperationContext::with_timeout(std::time::Duration::from_secs(600));
    let source_database = source.join(".codegraph/codegraph.db");
    let copied_existing_graph = source_database.is_file();
    if copied_existing_graph {
        std::fs::create_dir(root.join(".codegraph"))?;
        let connection =
            Connection::open_with_flags(&source_database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(std::time::Duration::from_secs(2))?;
        connection.execute(
            "VACUUM INTO ?1",
            [root
                .join(".codegraph/codegraph.db")
                .to_string_lossy()
                .as_ref()],
        )?;
    } else {
        index_store::initialize(root, &context)
            .map_err(|error| format!("{}: {}", error.0, error.1))?;
    }
    let database = Connection::open(root.join(".codegraph/codegraph.db"))?;
    let graph_before: Value = json!({
        "files":database.query_row("SELECT count(*) FROM files", [], |row| row.get::<_,i64>(0))?,
        "nodes":database.query_row("SELECT count(*) FROM nodes", [], |row| row.get::<_,i64>(0))?,
        "edges":database.query_row("SELECT count(*) FROM edges", [], |row| row.get::<_,i64>(0))?
    });
    drop(database);
    let selected: Vec<_> = files.iter().take(count).collect();
    if batch {
        if copied_existing_graph {
            return Err("Batch sample requires a fresh isolated index".into());
        }
        // These are plain files created by this probe in its own TempDir.
        for path in files.iter().skip(count) {
            std::fs::remove_file(root.join(path))?;
        }
    }
    let mut source_fingerprint = Vec::new();
    for path in &selected {
        source_fingerprint.extend_from_slice(path.as_bytes());
        source_fingerprint.push(0);
        source_fingerprint
            .extend_from_slice(harness_domain::hash(&std::fs::read(root.join(path))?).as_bytes());
    }
    let mut passes = Vec::new();
    for pass in 0..2 {
        let started = Instant::now();
        if batch {
            let report = index_store::rebuild_workspace(root, &context)
                .map_err(|error| format!("{}: {}", error.0, error.1))?;
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            passes.push(json!({"pass":pass+1,"elapsedMs":elapsed,"phaseTimingsMicros":report["phaseTimingsMicros"],"graphHash":graph_hash(root)?,"graphStatistics":graph_statistics(root)?,"workspaceReport":report}));
            continue;
        }
        let mut totals = BTreeMap::<String, u64>::new();
        let mut reports = Vec::new();
        for path in &selected {
            let file_started = Instant::now();
            let result = match index_store::sync_file_forced(root, path, &context) {
                Ok(result) => result,
                Err((code, message)) => {
                    let diagnostic = index_store::take_sync_diagnostic(code, message);
                    let phases = diagnostic
                        .as_ref()
                        .and_then(|value| value.get("phaseTimingsMicros"))
                        .cloned();
                    json!({"path":path,"code":code,"message":message,"diagnostic":diagnostic,"phaseTimingsMicros":phases})
                }
            };
            if let Some(phases) = result["phaseTimingsMicros"].as_object() {
                for (phase, value) in phases {
                    *totals.entry(phase.clone()).or_default() += value.as_u64().unwrap_or(0);
                }
            }
            reports.push(json!({"path":path,"durationMs":file_started.elapsed().as_secs_f64()*1000.0,"result":result}));
        }
        passes.push(json!({"pass":pass+1,"elapsedMs":started.elapsed().as_secs_f64()*1000.0,"phaseTimingsMicros":totals,"files":reports,"graphHash":graph_hash(root)?}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"sourceRoot":source,"copiedSourceFiles":files.len(),"copiedSourceBytes":copied_bytes,
        "sampleFiles":selected.len(),"sampleSourceHash":harness_domain::hash(&source_fingerprint),"batchMode":batch,"resolutionMemoEnabled":std::env::var("HARNESS_INDEX_RESOLUTION_MEMO").as_deref()!=Ok("0"),"copiedExistingGraph":copied_existing_graph,"graphBefore":graph_before,"passes":passes,
        "limitations":["Batch mode uses the real workspace coordinator over only the selected source files; default mode publishes sequentially", "Isolated database and source copy; service credentials and .agents configuration are not copied", "The second pass sees changes made by the first pass in the isolated graph"]})
        )?
    );
    Ok(())
}
