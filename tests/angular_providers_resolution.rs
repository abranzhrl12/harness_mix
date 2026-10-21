#![cfg(windows)]

use agent_harness::adapters::index_store::sync_file;
use harness_domain::execution::OperationContext;
use rusqlite::Connection;
use std::{fs, path::Path};

const ENGINE: &str = "angular-static-provider-registration-subset-v1";

fn fixture(root: &Path) {
    fs::create_dir_all(root.join(".codegraph")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
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

#[test]
fn aliases_and_barrels_resolve_exact_imports_without_global_homonyms() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    for (path, source) in [
        (
            "src/token.ts",
            "export class ApiToken {}\nexport class LegacyToken {}\n",
        ),
        (
            "src/impl.ts",
            "export class ApiImpl {}\nexport class ExistingImpl {}\n",
        ),
        (
            "barrel.ts",
            "export { ApiToken as TOKEN, LegacyToken as LEGACY } from './src/token';\nexport { ApiImpl as Impl, ExistingImpl as Existing } from './src/impl';\n",
        ),
        ("decoy.ts", "export class TOKEN {}\nexport class Impl {}\n"),
        (
            "app.ts",
            "import { NgModule } from '@angular/core';\nimport { TOKEN as Port, LEGACY, Impl as Adapter, Existing as OldAdapter } from './barrel';\n@NgModule({providers:[{provide:Port,useClass:Adapter},{provide:LEGACY,useExisting:OldAdapter}]}) export class AppModule {}\n",
        ),
    ] {
        fs::write(root.path().join(path), source).unwrap();
        let result = sync(root.path(), path);
        assert_eq!(result["committed"], true, "{path}: {result}");
    }
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        4
    );
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}' AND json_extract(metadata,'$.relationship')='token'"
            )
        ),
        2
    );
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}' AND json_extract(metadata,'$.relationship')='implementation'"
            )
        ),
        2
    );
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE s.file_path='app.ts' AND t.file_path='decoy.ts' AND json_extract(e.metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}' AND json_extract(metadata,'$.runtimeProviderSelectionEvaluated')=0"
            )
        ),
        4
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE s.file_path='app.ts' AND e.kind='contains' AND t.signature LIKE 'Angular provider registration %'"
        ),
        2
    );
}

#[test]
fn missing_provider_classes_retry_after_add_delete_and_readd() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::write(
        root.path().join("app.ts"),
        "import { NgModule } from '@angular/core'; import { Token, Impl } from './service'; @NgModule({providers:[{provide:Token,useClass:Impl}]}) export class AppModule {}",
    )
    .unwrap();
    let initial = sync(root.path(), "app.ts");
    assert_eq!(initial["committed"], true, "{initial}");
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM unresolved_refs WHERE reference_kind IN ('angular_provider_token','angular_provider_implementation') AND status='pending'"
            )
        ),
        2
    );
    fs::write(
        root.path().join("service.ts"),
        "export class Token {}\nexport class Impl {}\n",
    )
    .unwrap();
    let added = sync(root.path(), "service.ts");
    assert_eq!(added["committed"], true, "{added}");
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        2
    );
    fs::remove_file(root.path().join("service.ts")).unwrap();
    let deleted = sync(root.path(), "service.ts");
    assert_eq!(deleted["committed"], true, "{deleted}");
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_kind IN ('angular_provider_token','angular_provider_implementation') AND status='pending'"
        ),
        2
    );
    fs::write(
        root.path().join("service.ts"),
        "export class Token {}\nexport class Impl {}\n",
    )
    .unwrap();
    let restored = sync(root.path(), "service.ts");
    assert_eq!(restored["committed"], true, "{restored}");
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        2
    );
}

#[test]
fn newly_ambiguous_barrel_export_is_invalidated_and_recovers_when_removed() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    for (path, source) in [
        ("left.ts", "export class Token {}\nexport class Impl {}\n"),
        ("right.ts", "export class Other {}\n"),
        ("barrel.ts", "export * from './left';\n"),
        (
            "app.ts",
            "import { NgModule } from '@angular/core'; import { Token, Impl } from './barrel'; @NgModule({providers:[{provide:Token,useClass:Impl}]}) export class AppModule {}",
        ),
    ] {
        fs::write(root.path().join(path), source).unwrap();
        let result = sync(root.path(), path);
        assert_eq!(result["committed"], true, "{path}: {result}");
    }
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        2
    );
    fs::write(root.path().join("right.ts"), "export class Token {}\n").unwrap();
    let added = sync(root.path(), "right.ts");
    assert_eq!(added["committed"], true, "{added}");
    // A homonym in a module not reexported by the barrel cannot make its
    // explicit import ambiguous. Only the subsequent star branch does that.
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}' AND json_extract(metadata,'$.relationship')='token'"
            )
        ),
        1
    );
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}' AND json_extract(metadata,'$.relationship')='implementation'"
            )
        ),
        1
    );
    fs::write(
        root.path().join("barrel.ts"),
        "export * from './left';\nexport * from './right';\n",
    )
    .unwrap();
    let unique = sync(root.path(), "barrel.ts");
    assert_eq!(unique["committed"], true, "{unique}");
    // This is still ambiguous; removing the competing export restores the exact target.
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}' AND json_extract(metadata,'$.relationship')='token'"
            )
        ),
        0
    );
    fs::write(root.path().join("barrel.ts"), "export * from './left';\n").unwrap();
    let recovered = sync(root.path(), "barrel.ts");
    assert_eq!(recovered["committed"], true, "{recovered}");
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        2
    );
}

#[test]
fn semantic_exclude_keeps_provider_relationships_pending() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    fs::create_dir_all(root.path().join(".agents")).unwrap();
    fs::write(
        root.path().join(".agents/harness.config.json"),
        r#"{"indexing":{"semanticExclude":["app.ts"]}}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("service.ts"),
        "export class Token {}\nexport class Impl {}\n",
    )
    .unwrap();
    sync(root.path(), "service.ts");
    fs::write(
        root.path().join("app.ts"),
        "import { NgModule } from '@angular/core'; import { Token, Impl } from './service'; @NgModule({providers:[{provide:Token,useClass:Impl}]}) export class AppModule {}",
    )
    .unwrap();
    let result = sync(root.path(), "app.ts");
    assert_eq!(result["committed"], true, "{result}");
    assert_eq!(
        count(
            root.path(),
            &format!(
                "SELECT count(*) FROM edges WHERE json_extract(metadata,'$.resolvedBy')='{ENGINE}'"
            )
        ),
        0
    );
    assert_eq!(
        count(
            root.path(),
            "SELECT count(*) FROM unresolved_refs WHERE reference_kind IN ('angular_provider_token','angular_provider_implementation') AND status='pending'"
        ),
        2
    );
}
