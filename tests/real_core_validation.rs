//! Opt-in acceptance on a fresh copy of this real codebase. Never opens its index.
#![cfg(windows)]
use agent_harness::adapters;
use harness_domain::contracts::Request;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::Instant};

fn copy_sources(source: &Path, target: &Path) -> usize {
    fs::create_dir_all(target).unwrap();
    let mut copied = 0;
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let metadata = fs::symlink_metadata(entry.path()).unwrap();
        assert!(
            !metadata.file_type().is_symlink(),
            "Source copy refuses links"
        );
        if metadata.is_dir() {
            copied += copy_sources(&entry.path(), &target.join(entry.file_name()));
        } else if entry.path().extension().is_some_and(|ext| ext == "rs") {
            fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
            copied += 1;
        }
    }
    copied
}

#[test]
#[ignore = "diagnostic: measures extraction separately on copied real sources; run with --release"]
fn real_core_extraction_cost() {
    assert!(
        !cfg!(debug_assertions),
        "This diagnostic must run with --release"
    );
    let original = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = tempfile::tempdir().unwrap();
    for relative in ["src", "crates/domain/src", "crates/application/src"] {
        copy_sources(&original.join(relative), &root.path().join(relative));
    }
    let mut files = Vec::new();
    let mut pending = vec![root.path().to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else if entry.path().extension().is_some_and(|e| e == "rs") {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    for round in 0..3 {
        let context = harness_domain::execution::OperationContext::default();
        let started = Instant::now();
        let mut read_us = 0;
        let mut extract_us = 0;
        let mut nodes = 0;
        let mut refs = 0;
        let mut bytes = 0;
        for path in &files {
            let read = Instant::now();
            let source = fs::read_to_string(path).unwrap();
            read_us += read.elapsed().as_micros();
            bytes += source.len();
            let relative = path
                .strip_prefix(root.path())
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            let extract = Instant::now();
            let result = adapters::index_extract::extract(&relative, &source, &context).unwrap();
            extract_us += extract.elapsed().as_micros();
            nodes += result["nodes"].as_array().unwrap().len();
            refs += result["unresolvedReferences"].as_array().unwrap().len();
        }
        println!(
            "{}",
            json!({"diagnostic":"extraction-only-release","round":round+1,
            "files":files.len(),"bytes":bytes,"nodes":nodes,"references":refs,
            "readMs":read_us as f64/1000.0,"extractMs":extract_us as f64/1000.0,
            "totalMs":started.elapsed().as_micros() as f64/1000.0,
            "includesSqlOrResolution":false})
        );
    }
    adapters::index_store::initialize(
        root.path(),
        &harness_domain::execution::OperationContext::default(),
    )
    .unwrap();
    let write_started = Instant::now();
    let mut attempts = 0;
    let mut resolved = 0;
    let mut slow = Vec::new();
    for path in &files {
        let relative = path
            .strip_prefix(root.path())
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        let started = Instant::now();
        let report = adapters::index_store::sync_file(
            root.path(),
            &relative,
            &harness_domain::execution::OperationContext::default(),
        )
        .unwrap();
        let elapsed = started.elapsed().as_micros() as f64 / 1000.0;
        attempts += report["referencesAttempted"].as_u64().unwrap_or(0);
        resolved += report["referencesResolved"].as_u64().unwrap_or(0);
        slow.push((relative, elapsed, report["referencesAttempted"].clone()));
    }
    let write_ms = write_started.elapsed().as_micros() as f64 / 1000.0;
    slow.sort_by(|a, b| b.1.total_cmp(&a.1));
    let noop_started = Instant::now();
    for path in &files {
        let relative = path
            .strip_prefix(root.path())
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        let report = adapters::index_store::sync_file(
            root.path(),
            &relative,
            &harness_domain::execution::OperationContext::default(),
        )
        .unwrap();
        assert_eq!(report["changed"], false);
    }
    let noop_ms = noop_started.elapsed().as_micros() as f64 / 1000.0;
    let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    let fk_started = Instant::now();
    for _ in &files {
        let violations: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check LIMIT 1)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!violations);
    }
    println!(
        "{}",
        json!({"diagnostic":"writer-structure-release","files":files.len(),
        "fullPerFileWriterMs":write_ms,"unchangedPerFileWriterMs":noop_ms,
        "foreignKeyChecksOnFinalGraphMs":fk_started.elapsed().as_micros() as f64/1000.0,
        "referenceAttempts":attempts,"referencesResolved":resolved,"slowestFiles":&slow[..slow.len().min(5)],
        "foreignKeyTimingIsActualPhaseShare":false,"profileChangesProductionBehavior":false})
    );
}

fn cli(exe: &Path, root: &Path, option: &str) -> (Value, u128) {
    let started = Instant::now();
    let output = Command::new(exe)
        .arg("--root")
        .arg(root)
        .arg(option)
        .env("PATH", "")
        .output()
        .unwrap();
    let elapsed = started.elapsed().as_millis();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "{option} returned no JSON: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    println!(
        "{}",
        json!({"operation":option,"elapsedMs":elapsed,"success":output.status.success(),
        "candidates":report["nativeCandidates"],"committed":report["changedFilesCommitted"],
        "remaining":report["nativeFilesRemaining"],"failure":report["failure"],
        "discoveryComplete":report["discoveryComplete"],"unchecked":report["uncheckedFiles"]})
    );
    assert!(output.status.success(), "{option} failed: {report}");
    (report, elapsed)
}

fn explore(app: &adapters::NativeApplication, symbol: &str, sync: bool) -> Value {
    app.execute(Request::Explore(
        serde_json::from_value(json!({
            "query":symbol,"syncIndex":sync,"requireMatchingFiles":true
        }))
        .unwrap(),
    ))
}

#[test]
#[ignore = "copies real core sources into a temporary project; run acceptance explicitly"]
fn real_core_copy_lifecycle_without_node() {
    let original = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = tempfile::tempdir().unwrap();
    let mut files = copy_sources(&original.join("src"), &root.path().join("src"));
    files += copy_sources(
        &original.join("crates/domain/src"),
        &root.path().join("crates/domain/src"),
    );
    files += copy_sources(
        &original.join("crates/application/src"),
        &root.path().join("crates/application/src"),
    );
    println!(
        "{}",
        json!({"realSourceFiles":files,"originalIndexOpened":false})
    );
    let exe = std::env::var_os("HARNESS_NATIVE_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_agent-harness").into());
    cli(&exe, root.path(), "--init-index");
    let (first, _) = cli(&exe, root.path(), "--sync");
    assert_eq!(first["selectedScopeApplied"], true);
    assert_eq!(first["changedFilesCommitted"], files);
    assert_eq!(first["uncheckedFiles"], 0);
    assert_eq!(first["discoveryComplete"], true);
    let app = adapters::application(root.path()).unwrap();
    let baseline = explore(&app, "resolve_exact", false);
    assert_eq!(baseline["state"], "ok", "{baseline}");
    assert_eq!(
        baseline["data"]["focus"]["status"], "resolved",
        "{baseline}"
    );
    assert!(
        !baseline["data"]["graph"]["edges"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    println!(
        "{}",
        json!({"realSymbol":"resolve_exact","status":baseline["state"],
        "edges":baseline["data"]["graph"]["edges"].as_array().unwrap().len()})
    );

    let probe = root.path().join("acceptance_probe.rs");
    fs::write(
        &probe,
        "pub fn acceptance_target() {}\npub fn acceptance_caller(){acceptance_target();}\n",
    )
    .unwrap();
    let added = explore(&app, "acceptance_target", true);
    assert_eq!(added["state"], "ok", "{added}");
    assert!(
        added["data"]["text"]
            .as_str()
            .unwrap()
            .contains("acceptance_caller")
    );
    fs::write(
        &probe,
        "pub fn acceptance_target() {}\npub fn acceptance_changed(){acceptance_target();}\n",
    )
    .unwrap();
    let changed = explore(&app, "acceptance_target", true);
    assert_eq!(changed["state"], "ok", "{changed}");
    assert!(
        changed["data"]["text"]
            .as_str()
            .unwrap()
            .contains("acceptance_changed")
    );
    fs::remove_file(&probe).unwrap();
    let (removed, _) = cli(&exe, root.path(), "--sync");
    assert_eq!(removed["changedFilesCommitted"], 1);
    let missing = explore(&app, "acceptance_target", false);
    assert_ne!(missing["data"]["focus"]["status"], "resolved");
    let (rebuilt, _) = cli(&exe, root.path(), "--reindex");
    assert_eq!(rebuilt["changedFilesCommitted"], files);
    let after = explore(&app, "resolve_exact", false);
    assert_eq!(after["state"], "ok", "{after}");
    println!(
        "{}",
        json!({"acceptance":"passed","addedModifiedDeleted":true,
        "rebuild":true,"realSourceFiles":files,"originalIndexOpened":false})
    );
}

#[test]
#[ignore = "CLI benchmark on isolated real-source copies; requires release binary and provider CLI"]
fn real_core_release_vs_provider_cli() {
    let original = Path::new(env!("CARGO_MANIFEST_DIR"));
    let native_exe = std::env::var_os("HARNESS_NATIVE_BINARY").expect("release binary required");
    let artifact = tempfile::tempdir().unwrap();
    let pinned_exe = artifact.path().join("agent-harness.exe");
    fs::copy(&native_exe, &pinned_exe).unwrap();
    let native_exe = pinned_exe.into_os_string();
    let provider_cli = std::env::var_os("HARNESS_CODEGRAPH_CLI").expect("provider CLI required");
    let mut native_index = Vec::new();
    let mut provider_index = Vec::new();
    let mut native_sync = Vec::new();
    let mut provider_sync = Vec::new();
    let run_provider = |root: &Path, operation: &str| {
        let started = Instant::now();
        let output = Command::new(&provider_cli)
            .arg(operation)
            .arg(root)
            .env("NODE_OPTIONS", "")
            .env("NODE_PATH", "")
            .output()
            .unwrap();
        let elapsed = started.elapsed().as_micros() as f64 / 1000.0;
        assert!(
            output.status.success(),
            "Provider {operation} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        elapsed
    };
    let run_native = |root: &Path, operation: &str| {
        let started = Instant::now();
        let output = Command::new(&native_exe)
            .arg("--root")
            .arg(root)
            .arg(operation)
            .env("PATH", "")
            .output()
            .unwrap();
        let elapsed = started.elapsed().as_micros() as f64 / 1000.0;
        assert!(
            output.status.success(),
            "Native {operation} failed: {}; report: {}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["selectedScopeApplied"], true, "{report}");
        elapsed
    };
    for round in 0..3 {
        let native = tempfile::tempdir().unwrap();
        let provider = tempfile::tempdir().unwrap();
        let mut files = 0;
        for relative in ["src", "crates/domain/src", "crates/application/src"] {
            files += copy_sources(&original.join(relative), &native.path().join(relative));
            copy_sources(&original.join(relative), &provider.path().join(relative));
        }
        // Both start with the same empty schema; no installer or user config runs.
        for root in [native.path(), provider.path()] {
            adapters::index_store::initialize(
                root,
                &harness_domain::execution::OperationContext::default(),
            )
            .unwrap();
        }
        if round % 2 == 0 {
            native_index.push(run_native(native.path(), "--sync"));
            provider_index.push(run_provider(provider.path(), "index"));
        } else {
            provider_index.push(run_provider(provider.path(), "index"));
            native_index.push(run_native(native.path(), "--sync"));
        }
        let counts = |root: &Path| {
            let db = rusqlite::Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
            db.query_row("SELECT (SELECT count(*) FROM files),(SELECT count(*) FROM nodes),(SELECT count(*) FROM edges)", [], |r| Ok((r.get::<_, i64>(0)?,r.get::<_, i64>(1)?,r.get::<_, i64>(2)?))).unwrap()
        };
        let n = counts(native.path());
        let p = counts(provider.path());
        assert_eq!(n.0, files as i64);
        assert_eq!(p.0, files as i64);
        println!(
            "{}",
            json!({"round":round+1,"files":files,"nativeCounts":n,"providerCounts":p,
            "nativeIndexMs":native_index.last(),"providerIndexMs":provider_index.last()})
        );
        for repeat in 0..3 {
            if (round + repeat) % 2 == 0 {
                native_sync.push(run_native(native.path(), "--sync"));
                provider_sync.push(run_provider(provider.path(), "sync"));
            } else {
                provider_sync.push(run_provider(provider.path(), "sync"));
                native_sync.push(run_native(native.path(), "--sync"));
            }
        }
    }
    let median = |values: &mut Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    let n_index = median(&mut native_index);
    let p_index = median(&mut provider_index);
    let n_sync = median(&mut native_sync);
    let p_sync = median(&mut provider_sync);
    println!(
        "{}",
        json!({"benchmark":"cli-release-vs-provider","indexSamples":3,"syncSamples":9,
        "nativeIndexMedianMs":n_index,"providerIndexMedianMs":p_index,
        "nativeNoopMedianMs":n_sync,"providerNoopMedianMs":p_sync,
        "nativeIndexSamplesMs":native_index,"providerIndexSamplesMs":provider_index,
        "nativeNoopSamplesMs":native_sync,"providerNoopSamplesMs":provider_sync,
        "nativeExecutableBytes":fs::metadata(native_exe).unwrap().len(),
        "compilationIncluded":false,"processStartupIncluded":true,"filesystemCache":"not flushed",
        "semanticParityClaimed":false})
    );
}

#[test]
#[ignore = "read-only parity review on isolated frozen source copies; requires native binary and provider CLI"]
fn real_core_graph_parity_review() {
    use std::collections::{BTreeMap, BTreeSet};
    #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
    struct AuditNode {
        id: String,
        name: String,
        kind: String,
        qualified: String,
        path: String,
        line: i64,
        column: i64,
    }
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct AuditEdge {
        source: String,
        target: String,
        kind: String,
        line: Option<i64>,
        column: Option<i64>,
    }
    struct AuditDb {
        nodes: BTreeMap<String, AuditNode>,
        edges: BTreeSet<AuditEdge>,
        metadata: BTreeMap<AuditEdge, Value>,
        pending: BTreeMap<(String, String, Option<i64>, Option<i64>), Vec<String>>,
        raw_edges: usize,
    }
    fn manifest(root: &Path) -> BTreeMap<String, String> {
        let mut paths = vec![root.to_owned()];
        let mut result = BTreeMap::new();
        while let Some(path) = paths.pop() {
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                let meta = fs::symlink_metadata(&path).unwrap();
                assert!(!meta.file_type().is_symlink());
                if meta.is_dir() {
                    paths.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let relative = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/");
                    result.insert(relative, harness_domain::hash(&fs::read(path).unwrap()));
                }
            }
        }
        result
    }
    fn load(root: &Path) -> AuditDb {
        let db = rusqlite::Connection::open_with_flags(
            root.join(".codegraph/codegraph.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
                | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .unwrap();
        db.execute_batch("BEGIN").unwrap();
        let mut statement = db.prepare("SELECT id,name,kind,qualified_name,file_path,start_line,start_column FROM nodes ORDER BY id LIMIT 10001").unwrap();
        let nodes = statement
            .query_map([], |r| {
                Ok(AuditNode {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    qualified: r.get(3)?,
                    path: r.get(4)?,
                    line: r.get(5)?,
                    column: r.get(6)?,
                })
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(nodes.len() <= 10000);
        let nodes = nodes
            .into_iter()
            .map(|node| (node.id.clone(), node))
            .collect::<BTreeMap<_, _>>();
        let mut statement = db
            .prepare(
                "SELECT source,target,kind,line,col,metadata FROM edges ORDER BY id LIMIT 50001",
            )
            .unwrap();
        let rows = statement
            .query_map([], |r| {
                Ok((
                    AuditEdge {
                        source: r.get(0)?,
                        target: r.get(1)?,
                        kind: r.get(2)?,
                        line: r.get(3)?,
                        column: r.get(4)?,
                    },
                    r.get::<_, Option<String>>(5)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(rows.len() <= 50000);
        let raw_edges = rows.len();
        let mut metadata = BTreeMap::new();
        let mut edges = BTreeSet::new();
        for (edge, data) in rows {
            assert!(nodes.contains_key(&edge.source) && nodes.contains_key(&edge.target));
            metadata.insert(
                edge.clone(),
                data.and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or(Value::Null),
            );
            edges.insert(edge);
        }
        let mut statement = db.prepare("SELECT from_node_id,reference_name,reference_kind,line,col FROM unresolved_refs LIMIT 50001").unwrap();
        let refs = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(refs.len() <= 50000);
        let mut pending = BTreeMap::<_, Vec<_>>::new();
        for (source, name, kind, line, column) in refs {
            let kind = if kind == "function_ref" {
                "references".to_owned()
            } else {
                kind
            };
            pending
                .entry((source, kind, line, column))
                .or_default()
                .push(name);
        }
        AuditDb {
            nodes,
            edges,
            metadata,
            pending,
            raw_edges,
        }
    }
    fn relevant(edge: &AuditEdge) -> bool {
        matches!(
            edge.kind.as_str(),
            "calls" | "references" | "imports" | "instantiates" | "navigates"
        )
    }
    let source = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let provider = tempfile::tempdir().unwrap();
    let original = Path::new(env!("CARGO_MANIFEST_DIR"));
    for relative in ["src", "crates/domain/src", "crates/application/src"] {
        copy_sources(&original.join(relative), &source.path().join(relative));
    }
    let frozen = manifest(source.path());
    copy_sources(source.path(), native.path());
    copy_sources(source.path(), provider.path());
    assert_eq!(manifest(native.path()), frozen);
    assert_eq!(manifest(provider.path()), frozen);
    for root in [native.path(), provider.path()] {
        adapters::index_store::initialize(
            root,
            &harness_domain::execution::OperationContext::default(),
        )
        .unwrap();
    }
    let exe = std::env::var_os("HARNESS_NATIVE_BINARY").expect("native binary required");
    let pinned = tempfile::tempdir().unwrap();
    let pinned_exe = pinned.path().join("agent-harness.exe");
    fs::copy(exe, &pinned_exe).unwrap();
    cli(&pinned_exe, native.path(), "--sync");
    let provider_cli = std::env::var_os("HARNESS_CODEGRAPH_CLI").expect("provider CLI required");
    let output = Command::new(&provider_cli)
        .arg("index")
        .arg(provider.path())
        .env("NODE_OPTIONS", "")
        .env("NODE_PATH", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Provider failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(manifest(native.path()), frozen);
    assert_eq!(manifest(provider.path()), frozen);
    let n = load(native.path());
    let p = load(provider.path());
    let ids_equal = n.nodes.keys().eq(p.nodes.keys());
    let identities_different = n
        .nodes
        .iter()
        .filter(|(id, node)| p.nodes.get(*id) != Some(*node))
        .count();
    println!(
        "{}",
        json!({"audit":"graph-parity-review","frozenFiles":frozen.len(),"sourceHashesEqual":true,"originalIndexOpened":false,"nativeNodes":n.nodes.len(),"providerNodes":p.nodes.len(),"nodeIdsEqual":ids_equal,"identityDifferences":identities_different,"nativeRawEdges":n.raw_edges,"providerRawEdges":p.raw_edges,"nativeDistinctOccurrences":n.edges.len(),"providerDistinctOccurrences":p.edges.len()})
    );
    assert!(
        ids_equal && identities_different == 0,
        "Cannot compare edges by node ID until node identity differences are classified"
    );
    let kinds = n
        .edges
        .iter()
        .chain(p.edges.iter())
        .map(|e| e.kind.clone())
        .collect::<BTreeSet<_>>();
    for kind in kinds {
        let a = n
            .edges
            .iter()
            .filter(|e| e.kind == kind)
            .collect::<BTreeSet<_>>();
        let b = p
            .edges
            .iter()
            .filter(|e| e.kind == kind)
            .collect::<BTreeSet<_>>();
        println!(
            "{}",
            json!({"audit":"edge-kind","kind":kind,"native":a.len(),"provider":b.len(),"shared":a.intersection(&b).count(),"providerOnly":b.difference(&a).count(),"nativeOnly":a.difference(&b).count(),"usedByOldHarness":matches!(kind.as_str(),"calls"|"references"|"imports"|"instantiates"|"navigates")})
        );
    }
    let pairs = |db: &AuditDb| {
        db.edges
            .iter()
            .filter(|e| relevant(e) && e.source != e.target)
            .map(|e| (e.source.clone(), e.target.clone()))
            .collect::<BTreeSet<_>>()
    };
    let np = pairs(&n);
    let pp = pairs(&p);
    let mut callers_different = 0;
    let mut callees_different = 0;
    for node in n
        .nodes
        .values()
        .filter(|node| matches!(node.kind.as_str(), "function" | "method"))
    {
        callers_different += usize::from(
            np.iter()
                .filter(|(_, b)| b == &node.id)
                .map(|(a, _)| a)
                .collect::<BTreeSet<_>>()
                != pp
                    .iter()
                    .filter(|(_, b)| b == &node.id)
                    .map(|(a, _)| a)
                    .collect::<BTreeSet<_>>(),
        );
        callees_different += usize::from(
            np.iter()
                .filter(|(a, _)| a == &node.id)
                .map(|(_, b)| b)
                .collect::<BTreeSet<_>>()
                != pp
                    .iter()
                    .filter(|(a, _)| a == &node.id)
                    .map(|(_, b)| b)
                    .collect::<BTreeSet<_>>(),
        );
    }
    println!(
        "{}",
        json!({"audit":"relevant-neighbor-pairs-before-tool-caps","native":np.len(),"provider":pp.len(),"shared":np.intersection(&pp).count(),"providerOnly":pp.difference(&np).count(),"nativeOnly":np.difference(&pp).count(),"functionMethodFocuses":n.nodes.values().filter(|node|matches!(node.kind.as_str(),"function"|"method")).count(),"callerSetsDifferent":callers_different,"calleeSetsDifferent":callees_different,"exhaustiveHistoricalHarnessUseClaimed":false})
    );
    let mut reasons = BTreeMap::<String, usize>::new();
    let mut engines = BTreeMap::<String, usize>::new();
    let missing = p
        .edges
        .difference(&n.edges)
        .filter(|e| relevant(e))
        .collect::<Vec<_>>();
    let pending_for = |edge: &AuditEdge| {
        n.pending
            .get(&(
                edge.source.clone(),
                edge.kind.clone(),
                edge.line,
                edge.column,
            ))
            .cloned()
            .unwrap_or_default()
    };
    for edge in &missing {
        let names = pending_for(edge);
        let reason = if names.iter().any(|name| name.contains("::")) {
            "scoped-qualified-pending"
        } else if names.iter().any(|name| name.contains('.')) {
            "receiver-member-pending"
        } else if !names.is_empty() {
            "bare-name-pending"
        } else {
            "no-matching-pending-occurrence"
        };
        *reasons.entry(reason.to_owned()).or_default() += 1;
        let engine = p
            .metadata
            .get(*edge)
            .and_then(|v| v["resolvedBy"].as_str())
            .unwrap_or("unmarked");
        *engines.entry(engine.to_owned()).or_default() += 1;
    }
    println!(
        "{}",
        json!({"audit":"provider-only-relevant-occurrences","classifications":reasons,"providerResolutionLabels":engines,"classificationProvesCorrectness":false})
    );
    for edge in missing.iter().take(12) {
        println!(
            "{}",
            json!({"audit":"provider-only-example","source":p.nodes[&edge.source],"target":p.nodes[&edge.target],"kind":edge.kind,"line":edge.line,"column":edge.column,"nativePendingNames":pending_for(edge),"providerMetadata":p.metadata.get(*edge)})
        );
    }
    for edge in n.edges.difference(&p.edges).filter(|e| relevant(e)).take(5) {
        println!(
            "{}",
            json!({"audit":"native-only-example","source":n.nodes[&edge.source],"target":n.nodes[&edge.target],"kind":edge.kind,"line":edge.line,"column":edge.column,"nativeMetadata":n.metadata.get(edge)})
        );
    }
    let app = adapters::application(native.path()).unwrap();
    for name in ["plan_workspace", "sync_workspace_all", "resolve_exact"] {
        let targets = n
            .nodes
            .values()
            .filter(|node| node.name == name)
            .collect::<Vec<_>>();
        if targets.len() != 1 {
            println!(
                "{}",
                json!({"audit":"spot-check-skipped","symbol":name,"definitions":targets.len()})
            );
            continue;
        }
        let target = targets[0];
        for direction in ["callers", "callees"] {
            let native_direction = if direction == "callers" {
                "inbound"
            } else {
                "outbound"
            };
            let result = app.execute(Request::Explore(serde_json::from_value(json!({"query":name,"symbol":name,"direction":native_direction,"maxDepth":1,"maxFiles":1,"maxChars":1000,"requireMatchingFiles":true})).unwrap()));
            assert_eq!(result["state"], "ok", "{result}");
            let native_neighbors = result["data"]["graph"]["edges"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|edge| {
                    let source = edge["source"].as_str()?;
                    let destination = edge["target"].as_str()?;
                    if direction == "callers" && destination == target.id && source != target.id {
                        Some(source.to_owned())
                    } else if direction == "callees"
                        && source == target.id
                        && destination != target.id
                    {
                        Some(destination.to_owned())
                    } else {
                        None
                    }
                })
                .collect::<BTreeSet<_>>();
            let output = Command::new(&provider_cli)
                .args([
                    direction,
                    name,
                    "--json",
                    "--file",
                    &target.path,
                    "--limit",
                    "20",
                    "--path",
                ])
                .arg(provider.path())
                .env("NODE_OPTIONS", "")
                .env("NODE_PATH", "")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["filteredOut"], false, "{value}");
            let groups = value["definitions"].as_array().unwrap();
            assert_eq!(groups.len(), 1, "{value}");
            let provider_neighbors = groups[0][direction]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| node["id"].as_str().unwrap().to_owned())
                .collect::<BTreeSet<_>>();
            let summary = |ids: Vec<&String>| {
                ids.into_iter().take(6).map(|id|json!({"name":p.nodes[id].name,"path":p.nodes[id].path,"line":p.nodes[id].line})).collect::<Vec<_>>()
            };
            println!(
                "{}",
                json!({"audit":"actual-tool-spot-check","symbol":name,"path":target.path,"direction":direction,"nativeNeighbors":native_neighbors.len(),"providerNeighbors":provider_neighbors.len(),"sameNeighborSet":native_neighbors==provider_neighbors,"nativeTruncated":result["data"]["graph"]["truncated"],"providerTruncated":value["truncated"],"providerOnlyExamples":summary(provider_neighbors.difference(&native_neighbors).collect()),"nativeOnlyExamples":summary(native_neighbors.difference(&provider_neighbors).collect())})
            );
        }
    }
    println!(
        "{}",
        json!({"audit":"review-completed","semanticParityClaimed":false,"sourceCorpus":"current Rust core only; not all historical user projects or languages"})
    );
}
