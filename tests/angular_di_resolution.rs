#![cfg(windows)]

use agent_harness::adapters::index_store::{sync_file, sync_file_with_tsconfig};
use harness_domain::execution::OperationContext;
use rusqlite::Connection;
use std::{fs, path::Path};

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

fn sync(root: &Path, path: &str) -> serde_json::Value {
    sync_file(root, path, &OperationContext::default()).unwrap()
}

fn count(root: &Path, query: &str) -> i64 {
    Connection::open(root.join(".codegraph/codegraph.db"))
        .unwrap()
        .query_row(query, [], |row| row.get(0))
        .unwrap()
}

const APP: &str = "import { Component } from '@angular/core';\nimport { Dep } from './dep';\n@Component({ template: '' })\nexport class Consumer { constructor(private readonly dep: Dep) {} }\n";

#[test]
fn constructor_token_retries_after_add_delete_and_readd() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("app.ts"), APP).unwrap();

    let app = sync(root.path(), "app.ts");
    assert_eq!(app["committed"], true, "{app}");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_kind='angular_injects' AND reference_name='Dep' AND status='pending'"
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='angular-token-dependency-subset-v1'"
        ),
        0
    );

    fs::write(root.path().join("dep.ts"), "export class Dep {}\n").unwrap();
    let added = sync(root.path(), "dep.ts");
    assert_eq!(added["committed"], true, "{added}");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='references' AND s.file_path='app.ts' AND t.file_path='dep.ts' AND json_extract(e.metadata,'$.resolvedBy')='angular-token-dependency-subset-v1'"
        ),
        1
    );
    let (form, imported, contributors): (String, String, String) = Connection::open(
        root.path().join(".codegraph/codegraph.db"),
    )
    .unwrap()
    .query_row(
        "SELECT json_extract(metadata,'$.tokenForm'),json_extract(metadata,'$.imported'),json_extract(metadata,'$.exportContributors') FROM edges WHERE json_extract(metadata,'$.resolvedBy')='angular-token-dependency-subset-v1'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .unwrap();
    assert_eq!(form, "constructor_type");
    assert_eq!(imported, "Dep");
    assert!(contributors.contains("dep.ts"));

    fs::remove_file(root.path().join("dep.ts")).unwrap();
    let deleted = sync(root.path(), "dep.ts");
    assert_eq!(deleted["committed"], true, "{deleted}");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='angular-token-dependency-subset-v1'"
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_kind='angular_injects' AND reference_name='Dep' AND status='pending'"
        ),
        1
    );

    fs::write(root.path().join("dep.ts"), "export class Dep {}\n").unwrap();
    sync(root.path(), "dep.ts");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='angular-token-dependency-subset-v1'"
        ),
        1
    );
}

#[test]
fn reexport_star_ambiguity_invalidates_and_recovers_token_dependency() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    for (path, source) in [
        ("left.ts", "export class Dep {}\n"),
        ("right.ts", "export class Other {}\n"),
        (
            "barrel.ts",
            "export * from './left';\nexport * from './right';\n",
        ),
        (
            "app.ts",
            "import { Component } from '@angular/core';\nimport { Dep } from './barrel';\n@Component({ template: '' })\nexport class Consumer { constructor(private readonly dep: Dep) {} }\n",
        ),
    ] {
        fs::write(root.path().join(path), source).unwrap();
        let result = sync(root.path(), path);
        assert_eq!(result["committed"], true, "{path}: {result}");
    }
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='angular-token-dependency-subset-v1'"
        ),
        1
    );

    fs::write(root.path().join("right.ts"), "export class Dep {}\n").unwrap();
    let ambiguous = sync(root.path(), "right.ts");
    assert_eq!(ambiguous["committed"], true, "{ambiguous}");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='angular-token-dependency-subset-v1'"
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_kind='angular_injects' AND reference_name='Dep' AND status='pending'"
        ),
        1
    );

    fs::remove_file(root.path().join("right.ts")).unwrap();
    let recovered = sync(root.path(), "right.ts");
    assert_eq!(recovered["committed"], true, "{recovered}");
    // A barrel still exporting a missing module is not proven unambiguous.
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='angular-token-dependency-subset-v1'"
        ),
        0
    );
    fs::write(root.path().join("barrel.ts"), "export * from './left';\n").unwrap();
    sync(root.path(), "barrel.ts");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='angular-token-dependency-subset-v1'"
        ),
        1
    );
}

#[test]
fn missing_or_unsupported_import_never_uses_global_class_homonym() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(root.path().join("decoy.ts"), "export class Dep {}\n").unwrap();
    sync(root.path(), "decoy.ts");
    fs::write(
        root.path().join("app.ts"),
        "import { Component } from '@angular/core';\nimport { Dep } from 'external-package';\n@Component({ template: '' })\nexport class Consumer { constructor(private readonly dep: Dep) {} }\n",
    )
    .unwrap();
    let result = sync(root.path(), "app.ts");
    assert_eq!(result["committed"], true, "{result}");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='references' AND s.file_path='app.ts' AND t.name='Dep'"
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_kind='angular_injects' AND reference_name='Dep' AND status='pending'"
        ),
        1
    );

    fs::write(
        root.path().join("app.ts"),
        "import { Component } from '@angular/core';\nimport { Dep } from './decoy';\n@Component({ template: '' })\nexport class Consumer<Dep> { constructor(private readonly dep: Dep) {} }\n",
    )
    .unwrap();
    let shadowed = sync(root.path(), "app.ts");
    assert_eq!(shadowed["committed"], true, "{shadowed}");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='references' AND s.file_path='app.ts' AND t.file_path='decoy.ts'"
        ),
        0
    );
}

#[test]
fn config_only_change_retries_pending_angular_alias() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/dep.ts"), "export class Dep {}\n").unwrap();
    sync(root.path(), "src/dep.ts");
    fs::write(
        root.path().join("app.ts"),
        "import { Component } from '@angular/core';\nimport { Dep } from '@dep';\n@Component({ template: '' })\nexport class Consumer { constructor(private readonly dep: Dep) {} }\n",
    )
    .unwrap();
    fs::write(
        root.path().join("tsconfig.json"),
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@dep":["missing"]}}}"#,
    )
    .unwrap();
    let initial = sync_file_with_tsconfig(
        root.path(),
        "app.ts",
        "tsconfig.json",
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(initial["committed"], true, "{initial}");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='angular-token-dependency-subset-v1'"
        ),
        0
    );

    fs::write(
        root.path().join("tsconfig.json"),
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@dep":["src/dep"]}}}"#,
    )
    .unwrap();
    let refreshed = sync_file_with_tsconfig(
        root.path(),
        "app.ts",
        "tsconfig.json",
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(refreshed["committed"], true, "{refreshed}");
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='references' AND s.file_path='app.ts' AND t.file_path='src/dep.ts' AND json_extract(e.metadata,'$.resolvedBy')='angular-token-dependency-subset-v1'"
        ),
        1
    );
}
