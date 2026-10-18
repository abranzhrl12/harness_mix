//! Opt-in A/B on frozen Rust sources; only temporary indexes are opened.
#![cfg(windows)]
use std::{fs, path::Path, process::Command, time::Instant};

fn copy_sources(source: &Path, target: &Path) -> usize {
    fs::create_dir_all(target).unwrap();
    let mut count = 0;
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let metadata = fs::symlink_metadata(entry.path()).unwrap();
        assert!(!metadata.file_type().is_symlink());
        if metadata.is_dir() {
            count += copy_sources(&entry.path(), &target.join(entry.file_name()));
        } else if entry.path().extension().is_some_and(|ext| ext == "rs") {
            fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
            count += 1;
        }
    }
    count
}

fn snapshot(root: &Path) -> Vec<Vec<String>> {
    let db = rusqlite::Connection::open_with_flags(
        root.join(".codegraph/codegraph.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    [
        "SELECT json_array(id,name,kind,qualified_name,file_path,start_line) FROM nodes ORDER BY id",
        "SELECT json_array(source,target,kind,line,col,metadata,provenance) FROM edges ORDER BY source,target,kind,line,col",
        "SELECT json_array(from_node_id,reference_name,reference_kind,line,col,file_path,language) FROM unresolved_refs ORDER BY from_node_id,reference_name,reference_kind,line,col",
    ].iter().map(|query| db.prepare(query).unwrap().query_map([], |r| r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap()).collect()
}

#[test]
#[ignore = "requires HARNESS_POSITION_BASELINE_BINARY; compares frozen temporary indexes"]
fn frozen_native_position_ab() {
    let baseline =
        std::env::var_os("HARNESS_POSITION_BASELINE_BINARY").expect("baseline binary required");
    let candidate = env!("CARGO_BIN_EXE_agent-harness");
    let original = Path::new(env!("CARGO_MANIFEST_DIR"));
    let frozen = tempfile::tempdir().unwrap();
    let folders = ["src", "crates/domain/src", "crates/application/src"];
    let mut files = 0;
    for folder in folders {
        files += copy_sources(&original.join(folder), &frozen.path().join(folder));
    }
    let mut times = [Vec::<f64>::new(), Vec::<f64>::new()];
    let mut oracle = None;
    for round in 0..3 {
        for engine in if round % 2 == 0 { [0, 1] } else { [1, 0] } {
            let root = tempfile::tempdir().unwrap();
            for folder in folders {
                copy_sources(&frozen.path().join(folder), &root.path().join(folder));
            }
            let exe = if engine == 0 {
                Path::new(&baseline)
            } else {
                Path::new(candidate)
            };
            let init = Command::new(exe)
                .arg("--root")
                .arg(root.path())
                .arg("--init-index")
                .output()
                .unwrap();
            assert!(
                init.status.success(),
                "{}",
                String::from_utf8_lossy(&init.stderr)
            );
            let started = Instant::now();
            let output = Command::new(exe)
                .arg("--root")
                .arg(root.path())
                .arg("--reindex")
                .output()
                .unwrap();
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(
                output.status.success(),
                "{report}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(report["selectedScopeApplied"], true);
            let state = snapshot(root.path());
            if let Some(expected) = &oracle {
                assert_eq!(
                    &state, expected,
                    "semantic graph changed, engine {engine}, round {round}"
                );
            } else {
                oracle = Some(state);
            }
            times[engine].push(elapsed);
            println!(
                "{}",
                serde_json::json!({"probe":"frozen-native-position-ab","engine":if engine==0 {"before"} else {"after"},"round":round+1,"files":files,"elapsedMs":elapsed,"sameGraph":true})
            );
        }
    }
    let samples = times.clone();
    for time in &mut times {
        time.sort_by(f64::total_cmp);
    }
    println!(
        "{}",
        serde_json::json!({"probe":"frozen-native-position-summary","files":files,"beforeSamplesMs":samples[0],"afterSamplesMs":samples[1],"beforeMedianMs":times[0][1],"afterMedianMs":times[1][1],"sameNodesEdgesPendingRefs":true,"productionIndexOpened":false})
    );
}
