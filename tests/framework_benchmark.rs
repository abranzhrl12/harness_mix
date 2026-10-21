#![cfg(windows)]

//! Opt-in, bounded timing probe for framework extraction and incremental sync.
//! Timings exclude fixture and index creation; no production project is opened.

use agent_harness::adapters::{index_extract, index_store};
use harness_domain::execution::OperationContext;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{fs, hint::black_box, path::Path, time::Instant};

const ENGINE_VUE: &str = "vue-named-composable-subset-v1";
const ENGINE_ANGULAR: &str = "angular-token-dependency-subset-v1";
const MAX_ITERATIONS: usize = 20;

#[derive(Clone, Copy)]
struct Size {
    name: &'static str,
    references: usize,
    padding_lines: usize,
}

const SIZES: [Size; 3] = [
    Size {
        name: "small",
        references: 1,
        padding_lines: 0,
    },
    Size {
        name: "medium",
        references: 16,
        padding_lines: 512,
    },
    Size {
        name: "large",
        references: 64,
        padding_lines: 4096,
    },
];

fn iterations() -> usize {
    let value = std::env::var("HARNESS_FRAMEWORK_BENCH_ITERATIONS")
        .ok()
        .map(|raw| {
            raw.parse::<usize>().unwrap_or_else(|_| {
                panic!("HARNESS_FRAMEWORK_BENCH_ITERATIONS must be an integer from 1 to {MAX_ITERATIONS}")
            })
        })
        .unwrap_or(3);
    assert!(
        (1..=MAX_ITERATIONS).contains(&value),
        "HARNESS_FRAMEWORK_BENCH_ITERATIONS must be from 1 to {MAX_ITERATIONS}"
    );
    value
}

fn padding(lines: usize) -> String {
    let mut text = String::with_capacity(lines.saturating_mul(24));
    for line in 0..lines {
        text.push_str("// deterministic benchmark filler ");
        text.push_str(&line.to_string());
        text.push('\n');
    }
    text
}

fn sources(framework: &str, size: Size) -> (String, String, &'static str) {
    let fill = padding(size.padding_lines);
    if framework == "vue" {
        let imports = (0..size.references)
            .map(|i| format!("useComposable{i}"))
            .collect::<Vec<_>>()
            .join(",\n  ");
        let calls = (0..size.references)
            .map(|i| format!("    useComposable{i}();"))
            .collect::<Vec<_>>()
            .join("\n");
        let target = (0..size.references)
            .map(|i| format!("export function useComposable{i}() {{}}"))
            .collect::<Vec<_>>()
            .join("\n");
        let importer = format!(
            "<script setup lang=\"ts\">\nimport {{\n  {imports}\n}} from './bridge';\n{fill}export function run() {{\n{calls}\n}}\n</script>\n"
        );
        (importer, format!("{target}\n"), "App.vue")
    } else {
        let imported = (0..size.references)
            .map(|i| format!("Dep{i}"))
            .collect::<Vec<_>>()
            .join(",\n  ");
        let injected = (0..size.references)
            .map(|i| format!("    private readonly dep{i}: Dep{i},"))
            .collect::<Vec<_>>()
            .join("\n");
        let target = (0..size.references)
            .map(|i| format!("export class Dep{i} {{}}"))
            .collect::<Vec<_>>()
            .join("\n");
        let importer = format!(
            "import {{ Component }} from '@angular/core';\nimport {{\n  {imported}\n}} from './bridge';\n@Component({{ template: '' }})\nexport class Consumer {{\n{fill}  constructor(\n{injected}\n  ) {{}}\n}}\n"
        );
        (importer, format!("{target}\n"), "app.ts")
    }
}

fn feature_sources(feature: &str, size: Size) -> (String, String, &'static str) {
    let fill = padding(size.padding_lines);
    if feature == "vue_returned_methods" {
        let imports = (0..size.references)
            .map(|i| format!("useClock{i}"))
            .collect::<Vec<_>>()
            .join(",\n  ");
        let receivers = (0..size.references)
            .map(|i| format!("const clock{i} = useClock{i}();"))
            .collect::<Vec<_>>()
            .join("\n");
        let calls = (0..size.references)
            .map(|i| format!("    clock{i}.reset{i}();"))
            .collect::<Vec<_>>()
            .join("\n");
        let target = (0..size.references)
            .map(|i| {
                format!(
                    "export function useClock{i}() {{ function reset{i}() {{}} return {{ reset{i} }}; }}"
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let importer = format!(
            "<script setup lang=\"ts\">\nimport {{\n  {imports}\n}} from './bridge';\n{fill}{receivers}\nexport function run() {{\n{calls}\n}}\n</script>\n"
        );
        (importer, format!("{target}\n"), "App.vue")
    } else {
        let symbols = (0..size.references)
            .flat_map(|i| [format!("Token{i}"), format!("Impl{i}")])
            .collect::<Vec<_>>()
            .join(",\n  ");
        let providers = (0..size.references)
            .map(|i| format!("    {{ provide: Token{i}, useClass: Impl{i} }},"))
            .collect::<Vec<_>>()
            .join("\n");
        let target = (0..size.references)
            .flat_map(|i| {
                [
                    format!("export class Token{i} {{}}"),
                    format!("export class Impl{i} {{}}"),
                ]
            })
            .collect::<Vec<_>>()
            .join("\n");
        let importer = format!(
            "import {{ NgModule }} from '@angular/core';\nimport {{\n  {symbols}\n}} from './bridge';\n{fill}@NgModule({{\n  providers: [\n{providers}\n  ]\n}})\nexport class AppModule {{}}\n"
        );
        (importer, format!("{target}\n"), "app.ts")
    }
}

fn fixture(root: &Path) {
    fs::create_dir_all(root.join(".codegraph")).unwrap();
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0)",
        [],
    )
    .unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL").unwrap();
}

fn count(root: &Path, query: &str) -> i64 {
    Connection::open(root.join(".codegraph/codegraph.db"))
        .unwrap()
        .query_row(query, [], |row| row.get(0))
        .unwrap()
}

fn graph_fingerprint(root: &Path, importer: &str, edge_kind: &str, engine: &str) -> Vec<String> {
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    let mut statement = db
        .prepare(
            "SELECT json_array(t.file_path,t.name,e.kind,e.line,e.col,json_extract(e.metadata,'$.refName'),json_extract(e.metadata,'$.tokenForm'),json_extract(e.metadata,'$.exportContributors')) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE s.file_path=?1 AND e.kind=?2 AND json_extract(e.metadata,'$.resolvedBy')=?3 ORDER BY t.file_path,t.name,e.line,e.col",
        )
        .unwrap();
    statement
        .query_map(rusqlite::params![importer, edge_kind, engine], |row| {
            row.get::<_, String>(0)
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn report_sample(
    framework: &str,
    size: Size,
    operation: &str,
    sample: usize,
    elapsed_ms: f64,
    importer_bytes: usize,
    target_bytes: usize,
    details: Value,
) {
    println!(
        "{}",
        json!({
            "probe": "framework-extract-incremental-publication-v1",
            "framework": framework,
            "size": size.name,
            "expectedReferences": size.references,
            "sourceBytes": {"importer": importer_bytes, "target": target_bytes},
            "operation": operation,
            "sample": sample,
            "elapsedMs": elapsed_ms,
            "memory": "not measured by this test",
            "details": details
        })
    );
}

fn report_feature_sample(
    feature: &str,
    size: Size,
    operation: &str,
    sample: usize,
    elapsed_ms: f64,
    importer_bytes: usize,
    target_bytes: usize,
    details: Value,
) {
    println!(
        "{}",
        json!({
            "probe": "framework-feature-subsets-v1",
            "feature": feature,
            "size": size.name,
            "expectedBindings": size.references,
            "sourceBytes": {"importer": importer_bytes, "target": target_bytes},
            "operation": operation,
            "sample": sample,
            "elapsedMs": elapsed_ms,
            "memory": "not measured by this test",
            "details": details
        })
    );
}

fn assert_committed(result: &Value, path: &str) {
    assert_eq!(result["committed"], true, "{path}: {result}");
}

#[test]
#[ignore = "opt-in timing probe; creates isolated temporary CodeGraph indexes"]
fn framework_extract_and_incremental_publication_benchmark() {
    let repeat_count = iterations();

    for framework in ["vue", "angular"] {
        let engine = if framework == "vue" {
            ENGINE_VUE
        } else {
            ENGINE_ANGULAR
        };
        for size in SIZES {
            let (importer, target, importer_path) = sources(framework, size);
            let reference_kind = if framework == "vue" {
                "calls"
            } else {
                "angular_injects"
            };
            for sample in 1..=repeat_count {
                let context = OperationContext::default();
                let started = Instant::now();
                let extraction = black_box(
                    index_extract::extract(importer_path, black_box(&importer), &context)
                        .unwrap_or_else(|error| {
                            panic!("{framework}/{} extraction failed: {error:?}", size.name)
                        }),
                );
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                let refs = extraction["unresolvedReferences"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|reference| reference["referenceKind"] == reference_kind)
                    .count();
                assert_eq!(refs, size.references);
                report_sample(
                    framework,
                    size,
                    "extract_only",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    target.len(),
                    json!({"nodes":extraction["nodes"].as_array().map(Vec::len),"references":refs,"database":false}),
                );

                // Fixture creation and index schema setup stay outside timers.
                let root = tempfile::tempdir().unwrap();
                fixture(root.path());
                fs::write(root.path().join(importer_path), &importer).unwrap();

                let context = OperationContext::default();
                let started = Instant::now();
                let first = index_store::sync_file(root.path(), importer_path, &context).unwrap();
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_committed(&first, importer_path);
                let pending = count(
                    root.path(),
                    &format!(
                        "SELECT count(*) FROM unresolved_refs WHERE file_path='{importer_path}' AND reference_kind='{reference_kind}' AND status='pending'"
                    ),
                );
                assert_eq!(pending, size.references as i64);
                report_sample(
                    framework,
                    size,
                    "new_importer_publication_target_missing",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    target.len(),
                    json!({"committed":first["committed"],"pending":pending,"resolved":0,"referencesAttempted":first["referencesAttempted"]}),
                );

                // File creation itself is excluded; the timer covers target
                // publication, incremental retries, and transaction commit.
                fs::write(root.path().join("bridge.ts"), &target).unwrap();
                let context = OperationContext::default();
                let started = Instant::now();
                let arrival = index_store::sync_file(root.path(), "bridge.ts", &context).unwrap();
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_committed(&arrival, "bridge.ts");
                let resolved = count(
                    root.path(),
                    &format!(
                        "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE s.file_path='{importer_path}' AND e.kind='{}' AND t.file_path='bridge.ts' AND json_extract(e.metadata,'$.resolvedBy')='{engine}'",
                        if framework == "vue" {
                            "calls"
                        } else {
                            "references"
                        }
                    ),
                );
                assert_eq!(resolved, size.references as i64);
                assert_eq!(arrival["referencesResolved"], size.references);
                assert_eq!(
                    count(
                        root.path(),
                        &format!(
                            "SELECT count(*) FROM unresolved_refs WHERE file_path='{importer_path}' AND reference_kind='{reference_kind}' AND status='pending'"
                        ),
                    ),
                    0
                );
                let fingerprint = graph_fingerprint(
                    root.path(),
                    importer_path,
                    if framework == "vue" {
                        "calls"
                    } else {
                        "references"
                    },
                    engine,
                );
                assert_eq!(fingerprint.len(), size.references);
                report_sample(
                    framework,
                    size,
                    "target_publication_incremental_retry",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    target.len(),
                    json!({"committed":arrival["committed"],"resolvedEdges":resolved,"referencesResolved":arrival["referencesResolved"],"frameworkReferencesRechecked":arrival["frameworkReferencesRechecked"]}),
                );

                // This forces extraction/publication again. Sources and other
                // per-publication analysis caches are new for this transaction.
                let context = OperationContext::default();
                let started = Instant::now();
                let forced =
                    index_store::sync_file_forced(root.path(), importer_path, &context).unwrap();
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_committed(&forced, importer_path);
                let still_resolved = count(
                    root.path(),
                    &format!(
                        "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE s.file_path='{importer_path}' AND e.kind='{}' AND t.file_path='bridge.ts' AND json_extract(e.metadata,'$.resolvedBy')='{engine}'",
                        if framework == "vue" {
                            "calls"
                        } else {
                            "references"
                        }
                    ),
                );
                assert_eq!(still_resolved, size.references as i64);
                assert_eq!(forced["referencesResolved"], size.references);
                assert_eq!(
                    count(
                        root.path(),
                        &format!(
                            "SELECT count(*) FROM unresolved_refs WHERE file_path='{importer_path}' AND reference_kind='{reference_kind}' AND status='pending'"
                        ),
                    ),
                    0
                );
                assert_eq!(
                    graph_fingerprint(
                        root.path(),
                        importer_path,
                        if framework == "vue" {
                            "calls"
                        } else {
                            "references"
                        },
                        engine,
                    ),
                    fingerprint,
                    "forced re-extraction changed semantic graph for {framework}/{}",
                    size.name
                );
                report_sample(
                    framework,
                    size,
                    "forced_reextract_new_publication_caches",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    target.len(),
                    json!({"committed":forced["committed"],"resolvedEdges":still_resolved,"referencesResolved":forced["referencesResolved"],"sharedAnalysisCache":false}),
                );

                // An unchanged ordinary sync exercises the incremental no-op
                // path; it is reported separately and is not a resolver sample.
                let context = OperationContext::default();
                let started = Instant::now();
                let unchanged =
                    index_store::sync_file(root.path(), importer_path, &context).unwrap();
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(unchanged["changed"], false, "{unchanged}");
                assert_eq!(unchanged["extractionPerformed"], false, "{unchanged}");
                report_sample(
                    framework,
                    size,
                    "unchanged_importer_noop",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    target.len(),
                    json!({"changed":unchanged["changed"],"extractionPerformed":unchanged["extractionPerformed"],"resolverInvoked":false}),
                );

                let edited_target = format!("{target}// deterministic target edit\n");
                fs::write(root.path().join("bridge.ts"), &edited_target).unwrap();
                let context = OperationContext::default();
                let started = Instant::now();
                let edited = index_store::sync_file(root.path(), "bridge.ts", &context).unwrap();
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_committed(&edited, "bridge.ts");
                let edited_fingerprint = graph_fingerprint(
                    root.path(),
                    importer_path,
                    if framework == "vue" {
                        "calls"
                    } else {
                        "references"
                    },
                    engine,
                );
                assert_eq!(edited_fingerprint.len(), size.references);
                report_sample(
                    framework,
                    size,
                    "target_edit_and_recheck",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    edited_target.len(),
                    json!({"committed":edited["committed"],"resolvedEdges":edited_fingerprint.len(),"referencesResolved":edited["referencesResolved"]}),
                );

                fs::remove_file(root.path().join("bridge.ts")).unwrap();
                let context = OperationContext::default();
                let started = Instant::now();
                let deleted = index_store::sync_file(root.path(), "bridge.ts", &context).unwrap();
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_committed(&deleted, "bridge.ts");
                let pending_after_delete = count(
                    root.path(),
                    &format!(
                        "SELECT count(*) FROM unresolved_refs WHERE file_path='{importer_path}' AND reference_kind='{reference_kind}' AND status='pending'"
                    ),
                );
                assert_eq!(pending_after_delete, size.references as i64);
                assert_eq!(
                    count(
                        root.path(),
                        &format!(
                            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source WHERE s.file_path='{importer_path}' AND e.kind='{}' AND json_extract(e.metadata,'$.resolvedBy')='{engine}'",
                            if framework == "vue" {
                                "calls"
                            } else {
                                "references"
                            }
                        ),
                    ),
                    0
                );
                report_sample(
                    framework,
                    size,
                    "target_deletion_invalidation",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    edited_target.len(),
                    json!({"committed":deleted["committed"],"pending":pending_after_delete,"resolved":0}),
                );

                fs::write(root.path().join("bridge.ts"), &edited_target).unwrap();
                let context = OperationContext::default();
                let started = Instant::now();
                let readded = index_store::sync_file(root.path(), "bridge.ts", &context).unwrap();
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_committed(&readded, "bridge.ts");
                let readded_fingerprint = graph_fingerprint(
                    root.path(),
                    importer_path,
                    if framework == "vue" {
                        "calls"
                    } else {
                        "references"
                    },
                    engine,
                );
                assert_eq!(readded_fingerprint, edited_fingerprint);
                assert_eq!(
                    count(
                        root.path(),
                        &format!(
                            "SELECT count(*) FROM unresolved_refs WHERE file_path='{importer_path}' AND reference_kind='{reference_kind}' AND status='pending'"
                        ),
                    ),
                    0
                );
                report_sample(
                    framework,
                    size,
                    "target_readd_and_recovery",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    edited_target.len(),
                    json!({"committed":readded["committed"],"resolvedEdges":readded_fingerprint.len(),"pending":0}),
                );
            }
        }
    }
}

#[test]
#[ignore = "opt-in timing probe for returned-method and static-provider resolution"]
fn framework_returned_methods_and_providers_benchmark() {
    let iteration_count = iterations();
    for feature in ["vue_returned_methods", "angular_static_providers"] {
        let (engine, edge_kind) = if feature == "vue_returned_methods" {
            ("vue-composable-return-method-subset-v1", "calls")
        } else {
            (
                "angular-static-provider-registration-subset-v1",
                "references",
            )
        };
        for size in SIZES {
            let (importer, target, importer_path) = feature_sources(feature, size);
            for sample in 1..=iteration_count {
                let context = OperationContext::default();
                let started = Instant::now();
                let extraction = black_box(
                    index_extract::extract(importer_path, black_box(&importer), &context)
                        .unwrap_or_else(|error| {
                            panic!("{feature}/{} extraction failed: {error:?}", size.name)
                        }),
                );
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                let extraction_refs = extraction["unresolvedReferences"].as_array().unwrap();
                let extracted = if feature == "vue_returned_methods" {
                    extraction_refs
                        .iter()
                        .filter(|reference| {
                            reference["referenceKind"] == "calls"
                                && reference["referenceName"].as_str().is_some_and(|name| {
                                    name.starts_with("clock") && name.contains(".reset")
                                })
                        })
                        .count()
                } else {
                    extraction_refs
                        .iter()
                        .filter(|reference| {
                            matches!(
                                reference["referenceKind"].as_str(),
                                Some("angular_provider_token" | "angular_provider_implementation")
                            )
                        })
                        .count()
                };
                let expected_extracted = if feature == "vue_returned_methods" {
                    size.references
                } else {
                    2 * size.references
                };
                assert_eq!(extracted, expected_extracted, "{feature}/{}", size.name);
                report_feature_sample(
                    feature,
                    size,
                    "extract_only",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    target.len(),
                    json!({"specializedReferences":extracted,"nodes":extraction["nodes"].as_array().map(Vec::len),"database":false}),
                );

                // Keep setup outside all measured intervals.
                let root = tempfile::tempdir().unwrap();
                fixture(root.path());
                fs::write(root.path().join(importer_path), &importer).unwrap();

                let context = OperationContext::default();
                let started = Instant::now();
                let first = index_store::sync_file(root.path(), importer_path, &context).unwrap();
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_committed(&first, importer_path);
                let initial_edges = count(
                    root.path(),
                    &format!(
                        "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{engine}'"
                    ),
                );
                assert_eq!(initial_edges, 0);
                report_feature_sample(
                    feature,
                    size,
                    "new_importer_publication_target_missing",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    target.len(),
                    json!({"committed":first["committed"],"resolvedEdges":0,"referencesAttempted":first["referencesAttempted"]}),
                );

                fs::write(root.path().join("bridge.ts"), &target).unwrap();
                let context = OperationContext::default();
                let started = Instant::now();
                let arrived = index_store::sync_file(root.path(), "bridge.ts", &context).unwrap();
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_committed(&arrived, "bridge.ts");
                let resolved = count(
                    root.path(),
                    &format!(
                        "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source WHERE s.file_path='{importer_path}' AND e.kind='{edge_kind}' AND json_extract(e.metadata,'$.resolvedBy')='{engine}'"
                    ),
                );
                let expected_edges = if feature == "vue_returned_methods" {
                    size.references
                } else {
                    2 * size.references
                };
                assert_eq!(resolved, expected_edges as i64);
                if feature == "angular_static_providers" {
                    assert_eq!(
                        count(
                            root.path(),
                            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='angular-static-provider-registration-subset-v1' AND json_extract(metadata,'$.relationship')='token'"
                        ),
                        size.references as i64
                    );
                    assert_eq!(
                        count(
                            root.path(),
                            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='angular-static-provider-registration-subset-v1' AND json_extract(metadata,'$.relationship')='implementation'"
                        ),
                        size.references as i64
                    );
                }
                let fingerprint = graph_fingerprint(root.path(), importer_path, edge_kind, engine);
                assert_eq!(fingerprint.len(), expected_edges);
                report_feature_sample(
                    feature,
                    size,
                    "target_publication_incremental_retry",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    target.len(),
                    json!({"committed":arrived["committed"],"resolvedEdges":resolved,"referencesResolved":arrived["referencesResolved"]}),
                );

                let context = OperationContext::default();
                let started = Instant::now();
                let forced =
                    index_store::sync_file_forced(root.path(), importer_path, &context).unwrap();
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_committed(&forced, importer_path);
                let forced_fingerprint =
                    graph_fingerprint(root.path(), importer_path, edge_kind, engine);
                assert_eq!(forced_fingerprint, fingerprint);
                report_feature_sample(
                    feature,
                    size,
                    "forced_reextract_new_publication_caches",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    target.len(),
                    json!({"committed":forced["committed"],"resolvedEdges":forced_fingerprint.len(),"sharedAnalysisCache":false}),
                );

                let edited_target = format!("{target}// deterministic feature target edit\n");
                fs::write(root.path().join("bridge.ts"), &edited_target).unwrap();
                let context = OperationContext::default();
                let started = Instant::now();
                let edited = index_store::sync_file(root.path(), "bridge.ts", &context).unwrap();
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_committed(&edited, "bridge.ts");
                let edited_fingerprint =
                    graph_fingerprint(root.path(), importer_path, edge_kind, engine);
                assert_eq!(edited_fingerprint.len(), expected_edges);
                report_feature_sample(
                    feature,
                    size,
                    "target_edit_and_recheck",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    edited_target.len(),
                    json!({"committed":edited["committed"],"resolvedEdges":edited_fingerprint.len()}),
                );

                fs::remove_file(root.path().join("bridge.ts")).unwrap();
                let context = OperationContext::default();
                let started = Instant::now();
                let deleted = index_store::sync_file(root.path(), "bridge.ts", &context).unwrap();
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_committed(&deleted, "bridge.ts");
                assert_eq!(
                    count(
                        root.path(),
                        &format!(
                            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{engine}'"
                        )
                    ),
                    0
                );
                let pending = if feature == "vue_returned_methods" {
                    count(
                        root.path(),
                        "SELECT count(*) FROM unresolved_refs WHERE file_path='App.vue' AND reference_kind='calls' AND reference_name LIKE 'clock%.reset%' AND status='pending'",
                    )
                } else {
                    count(
                        root.path(),
                        "SELECT count(*) FROM unresolved_refs WHERE file_path='app.ts' AND reference_kind IN ('angular_provider_token','angular_provider_implementation') AND status='pending'",
                    )
                };
                assert_eq!(pending, expected_edges as i64);
                report_feature_sample(
                    feature,
                    size,
                    "target_deletion_invalidation",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    edited_target.len(),
                    json!({"committed":deleted["committed"],"pending":pending,"resolvedEdges":0}),
                );

                fs::write(root.path().join("bridge.ts"), &edited_target).unwrap();
                let context = OperationContext::default();
                let started = Instant::now();
                let readded = index_store::sync_file(root.path(), "bridge.ts", &context).unwrap();
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_committed(&readded, "bridge.ts");
                let readded_fingerprint =
                    graph_fingerprint(root.path(), importer_path, edge_kind, engine);
                assert_eq!(readded_fingerprint, edited_fingerprint);
                report_feature_sample(
                    feature,
                    size,
                    "target_readd_and_recovery",
                    sample,
                    elapsed_ms,
                    importer.len(),
                    edited_target.len(),
                    json!({"committed":readded["committed"],"resolvedEdges":readded_fingerprint.len(),"pending":0}),
                );
            }
        }
    }
}
