#![cfg(windows)]

use agent_harness::adapters::index_store::{sync_changed_paths, sync_file};
use harness_domain::{execution::OperationContext, hash};
use rusqlite::Connection;
use std::{collections::BTreeSet, fs};

fn initialized() -> (tempfile::TempDir, OperationContext) {
    let root = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    fs::create_dir(root.path().join(".codegraph")).unwrap();
    let db = Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    db.execute(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0)",
        [],
    )
    .unwrap();
    drop(db);
    (root, context)
}

fn database(root: &std::path::Path) -> Connection {
    Connection::open(root.join(".codegraph/codegraph.db")).unwrap()
}

#[test]
fn cp1252_csharp_indexes_symbols_and_preserves_the_raw_source_hash() {
    let (root, context) = initialized();
    let mut raw = b"// Acci".to_vec();
    raw.push(0xF3); // ó in Windows-1252
    raw.extend_from_slice(b"n y ni");
    raw.push(0xF1); // ñ in Windows-1252
    raw.extend_from_slice(b"ez\r\npublic class Propuesta { public void Buscar() {} }\r\n");
    fs::write(root.path().join("Propuesta.cs"), &raw).unwrap();

    let report = sync_file(root.path(), "Propuesta.cs", &context).unwrap();
    assert_eq!(report["changed"], true, "{report}");
    let db = database(root.path());
    let method_count: i64 = db
        .query_row(
            "SELECT count(*) FROM nodes WHERE file_path='Propuesta.cs' AND language='csharp' AND kind='method' AND name='Buscar'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(method_count, 1);
    let stored_hash: String = db
        .query_row(
            "SELECT content_hash FROM files WHERE path='Propuesta.cs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_hash, hash(&raw));
}

#[test]
fn duplicate_legacy_javascript_declarations_warn_without_blocking_other_files() {
    let (root, context) = initialized();
    fs::write(
        root.path().join("Script_SeguimientoLeads.js"),
        concat!(
            "var leads = []; var leads = window.leads || [];\n",
            "function actualizarLead() { var indice = 0; var indice = 1; return indice; }\n",
            "function actualizarLead() { return leads.length; }\n"
        ),
    )
    .unwrap();
    fs::write(
        root.path().join("Other.js"),
        "function unrelatedValidFile() { return true; }\n",
    )
    .unwrap();

    let result = sync_changed_paths(
        root.path(),
        &BTreeSet::from([
            "Script_SeguimientoLeads.js".to_owned(),
            "Other.js".to_owned(),
        ]),
        &context,
    )
    .unwrap();
    assert_eq!(result["changedFilesCommitted"], 2, "{result}");
    let db = database(root.path());
    for (path, symbol) in [
        ("Script_SeguimientoLeads.js", "actualizarLead"),
        ("Other.js", "unrelatedValidFile"),
    ] {
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM nodes WHERE file_path=?1 AND language='javascript' AND kind='function' AND name=?2",
                rusqlite::params![path, symbol],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count > 0, "missing {symbol} from {path}; {result}");
    }
    let indexed_files: i64 = db
        .query_row(
            "SELECT count(*) FROM files WHERE path IN ('Script_SeguimientoLeads.js','Other.js')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(indexed_files, 2, "{result}");
}
