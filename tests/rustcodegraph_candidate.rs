//! Opt-in adoption gate. The candidate runs only on a disposable project.
//! Differences are reported, not silently accepted as replacement semantics.
#![cfg(windows)]

use agent_harness::adapters::index_extract;
use harness_domain::execution::OperationContext;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const REVISION: &str = "cfd4d74132219f498d5ae611d7e94990a27c2c0d";

enum Probe {
    Nodes(&'static str),
    Anchor(&'static str),
    Reference(&'static str),
    Template(&'static str),
}

struct Case {
    path: &'static str,
    source: &'static str,
    probe: Probe,
    expected_count: usize,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            path: "vue/basic.vue",
            source: "<script lang=\"ts\">\nexport function basic() {}\n</script>\n",
            probe: Probe::Nodes("basic"),
            expected_count: 1,
        },
        Case {
            path: "vue/inline.vue",
            source: "<!-- 😀 --><script setup lang=\"ts\">export function inlineRun() {}</script>",
            probe: Probe::Anchor("inlineRun"),
            expected_count: 1,
        },
        Case {
            path: "vue/two-blocks.vue",
            source: "<script lang=\"ts\">\nfunction repeated() {}\n</script>\n<script setup lang=\"ts\">\nfunction repeated() {}\n</script>\n",
            probe: Probe::Nodes("repeated"),
            expected_count: 2,
        },
        Case {
            path: "vue/external.vue",
            source: "<script src=\"./missing.ts\">export function shouldNotExist() {}</script>",
            probe: Probe::Nodes("shouldNotExist"),
            expected_count: 0,
        },
        Case {
            path: "vue/template.vue",
            source: "<script setup>const ready = true;</script><template><OnlyTemplateCandidate /></template>",
            probe: Probe::Template("OnlyTemplateCandidate"),
            expected_count: 1,
        },
        Case {
            path: "razor/members.cshtml",
            source: "@functions {\n public int MemberCount() { return 1; }\n}\n",
            probe: Probe::Nodes("MemberCount"),
            expected_count: 1,
        },
        Case {
            path: "razor/qualified.cshtml",
            source: "@model App.Models.Person\n<div>Hello</div>\n",
            probe: Probe::Reference("App.Models.Person"),
            expected_count: 1,
        },
        Case {
            path: "razor/comment.cshtml",
            source: "@* @functions { public void Dummy() { CommentOnlyCall(); } } *@\n<div>ok</div>\n",
            probe: Probe::Reference("CommentOnlyCall"),
            expected_count: 0,
        },
    ]
}

fn run(binary: &Path, args: &[&str], root: &Path) -> std::process::Output {
    let mut child = Command::new(binary)
        .args(args)
        .arg(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("candidate exceeded 60 seconds on a tiny isolated fixture");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "candidate failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn candidate_nodes(db: &Connection, case: &Case, name: &str) -> Vec<Value> {
    let mut query = db.prepare("SELECT kind,start_line,start_column FROM nodes WHERE name=?1 AND (replace(file_path,char(92),'/')=?2 OR replace(file_path,char(92),'/') LIKE '%/' || ?2) AND kind IN ('function','method') ORDER BY start_line,start_column").unwrap();
    query.query_map(params![name, case.path], |row| Ok(json!({"kind":row.get::<_,String>(0)?,"line":row.get::<_,i64>(1)?,"column":row.get::<_,i64>(2)?}))).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
}

fn candidate_reference_count(db: &Connection, case: &Case, name: &str) -> usize {
    db.query_row("SELECT count(*) FROM unresolved_refs WHERE reference_name=?1 AND (replace(file_path,char(92),'/')=?2 OR replace(file_path,char(92),'/') LIKE '%/' || ?2)", params![name,case.path], |row| row.get::<_, i64>(0)).unwrap().try_into().unwrap()
}

fn diagnostics(db: &Connection, case: &Case) -> Value {
    let mut symbols = db.prepare("SELECT name,kind,start_line,start_column FROM nodes WHERE replace(file_path,char(92),'/')=?1 OR replace(file_path,char(92),'/') LIKE '%/' || ?1 ORDER BY start_line,start_column LIMIT 20").unwrap();
    let symbols = symbols.query_map([case.path], |row| Ok(json!({"name":row.get::<_,String>(0)?,"kind":row.get::<_,String>(1)?,"line":row.get::<_,i64>(2)?,"column":row.get::<_,i64>(3)?}))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    let mut refs = db.prepare("SELECT reference_name FROM unresolved_refs WHERE replace(file_path,char(92),'/')=?1 OR replace(file_path,char(92),'/') LIKE '%/' || ?1 LIMIT 20").unwrap();
    let refs = refs
        .query_map([case.path], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut files = db.prepare("SELECT language,errors FROM files WHERE replace(path,char(92),'/')=?1 OR replace(path,char(92),'/') LIKE '%/' || ?1 LIMIT 2").unwrap();
    let files = files.query_map([case.path], |row| {
        let errors = row.get::<_,Option<String>>(1)?.and_then(|text| serde_json::from_str::<Value>(&text).ok());
        Ok(json!({"language":row.get::<_,String>(0)?,"errorCount":errors.as_ref().and_then(Value::as_array).map(Vec::len)}))
    }).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    json!({"files":files,"symbols":symbols,"pendingReferenceNames":refs})
}

fn graph_observation(graph: &Value, case: &Case) -> Value {
    let node_rows = |name: &str| {
        graph["nodes"].as_array().unwrap().iter()
        .filter(|node| node["name"] == name && matches!(node["kind"].as_str(), Some("function"|"method")))
        .map(|node| json!({"kind":node["kind"],"line":node["startLine"],"column":node["startColumn"]})).collect::<Vec<_>>()
    };
    match case.probe {
        Probe::Nodes(name) => json!(node_rows(name).len()),
        Probe::Anchor(name) => node_rows(name)
            .first()
            .map(|row| json!({"line":row["line"],"column":row["column"]}))
            .unwrap_or(Value::Null),
        Probe::Reference(name) => {
            let own = graph["unresolvedReferences"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|reference| reference["referenceName"] == name)
                .count();
            json!(own)
        }
        Probe::Template(name) => {
            let own = graph["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node["kind"] == "module" && node["name"] == name);
            let referenced = graph["unresolvedReferences"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reference| reference["referenceName"] == name);
            json!(usize::from(own || referenced))
        }
    }
}

fn observations(db: &Connection, case: &Case) -> (Value, Value, Value) {
    let graph =
        index_extract::extract(case.path, case.source, &OperationContext::default()).unwrap();
    let candidate = match case.probe {
        Probe::Nodes(name) => json!(candidate_nodes(db, case, name).len()),
        Probe::Anchor(name) => candidate_nodes(db, case, name)
            .first()
            .map(|row| json!({"line":row["line"],"column":row["column"]}))
            .unwrap_or(Value::Null),
        Probe::Reference(name) => json!(candidate_reference_count(db, case, name)),
        Probe::Template(name) => json!(usize::from(candidate_reference_count(db, case, name) > 0)),
    };
    let expected = match case.probe {
        Probe::Anchor(name) => {
            let at = case.source.find(&format!("function {name}")).unwrap();
            let prefix = &case.source[..at];
            json!({"line":prefix.bytes().filter(|&byte|byte==b'\n').count()+1,"column":prefix.rsplit('\n').next().unwrap().encode_utf16().count()})
        }
        _ => json!(case.expected_count),
    };
    (graph_observation(&graph, case), candidate, expected)
}

#[test]
#[ignore = "requires a source-built pinned HARNESS_RUSTCODEGRAPH_BINARY; temporary index only"]
fn rustcodegraph_candidate_contract_audit() {
    let binary = fs::canonicalize(
        std::env::var_os("HARNESS_RUSTCODEGRAPH_BINARY").expect("candidate binary required"),
    )
    .unwrap();
    let version = Command::new(&binary).arg("--version").output().unwrap();
    assert!(version.status.success());
    let version = String::from_utf8(version.stdout).unwrap();
    assert_eq!(
        version.trim(),
        "1.2.10",
        "review a new candidate revision before comparing"
    );
    let binary_hash = harness_domain::hash(&fs::read(&binary).unwrap());
    let library_probe = fs::canonicalize(
        std::env::var_os("HARNESS_RUSTCODEGRAPH_PROBE_BINARY")
            .expect("isolated library probe binary required"),
    )
    .unwrap();
    let probe_version = Command::new(&library_probe)
        .arg("--version")
        .output()
        .unwrap();
    assert!(probe_version.status.success());
    assert_eq!(
        String::from_utf8(probe_version.stdout).unwrap().trim(),
        format!("rustcodegraph-probe {REVISION}")
    );
    let library_probe_hash = harness_domain::hash(&fs::read(&library_probe).unwrap());
    let root = tempfile::tempdir().unwrap();
    let cases = cases();
    for case in &cases {
        let path = root.path().join(case.path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, case.source).unwrap();
    }
    run(&binary, &["init", "-i", "--quiet"], root.path());
    let database = root.path().join(".rustcodegraph/rustcodegraph.db");
    let db = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    db.execute_batch("PRAGMA query_only=ON").unwrap();
    let mut gaps = 0;
    let mut library_gaps = 0;
    for case in &cases {
        let (harness, candidate, expected) = observations(&db, case);
        assert_eq!(harness, expected, "harness oracle changed: {}", case.path);
        // A healthy baseline is mandatory; differences must not conceal a
        // failed candidate build, invalid schema or empty index.
        if case.path == "vue/basic.vue" {
            assert_eq!(candidate, expected);
        }
        let compatible = candidate == expected;
        gaps += usize::from(!compatible);
        let output = run(&library_probe, &[case.path], &root.path().join(case.path));
        let library_graph: Value =
            serde_json::from_slice(&output.stdout).expect("library probe JSON");
        let library = graph_observation(&library_graph, case);
        let ids = library_graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|node| node["id"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let duplicate_ids = library_graph["nodes"].as_array().unwrap().len() - ids.len();
        let library_compatible = library == expected && duplicate_ids == 0;
        library_gaps += usize::from(!library_compatible);
        if case.path == "vue/basic.vue" {
            assert_eq!(library, expected, "library probe must be healthy");
        }
        println!(
            "{}",
            json!({"probe":"rustcodegraph-candidate-contract-v2","path":case.path,"expected":expected,"harness":harness,"candidateCli":candidate,"cliCompatible":compatible,"candidateLibrary":library,"libraryCompatible":library_compatible,"libraryDuplicateNodeIds":duplicate_ids,"libraryErrors":library_graph["errors"],"libraryReferenceNames":library_graph["unresolvedReferences"].as_array().unwrap().iter().map(|reference| &reference["referenceName"]).collect::<Vec<_>>(),"candidateDiagnostics":diagnostics(&db,case)})
        );
    }
    drop(db);
    let basic = &cases[0];
    fs::write(
        root.path().join(basic.path),
        basic.source.replace("basic()", "renamed()"),
    )
    .unwrap();
    run(&binary, &["sync", "--quiet"], root.path());
    let db = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert!(candidate_nodes(&db, basic, "basic").is_empty());
    assert_eq!(candidate_nodes(&db, basic, "renamed").len(), 1);
    println!(
        "{}",
        json!({"probe":"rustcodegraph-candidate-summary-v2","expectedRevision":REVISION,"binarySha256":binary_hash,"libraryProbeSha256":library_probe_hash,"candidateVersion":version.trim(),"cases":cases.len(),"cliGaps":gaps,"libraryGaps":library_gaps,"cliReplacementGatePassed":gaps==0,"libraryAdoptionGatePassed":library_gaps==0,"incrementalCliRenamePassed":true,"productionIndexOpened":false,"performanceCompared":false})
    );
}
