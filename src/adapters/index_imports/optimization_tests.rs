use super::*;
use rusqlite::StatementStatus;

const SYN: &str = "syn-module-subset-v1";
const RA: &str = "ra-module-subset-v1";

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE nodes(id TEXT PRIMARY KEY,file_path TEXT,language TEXT);
         CREATE TABLE edges(source TEXT,target TEXT,kind TEXT,line INTEGER,col INTEGER,metadata TEXT,provenance TEXT);
         CREATE TABLE unresolved_refs(id INTEGER PRIMARY KEY,from_node_id TEXT,reference_name TEXT,reference_kind TEXT,line INTEGER,col INTEGER,candidates TEXT,file_path TEXT,language TEXT,status TEXT,name_tail TEXT);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO edges VALUES('syn','target','calls',8,9,'{malformed',NULL)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO nodes VALUES('syn','src/syn.rs','rust'),('ra','src/ra.rs','rust')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO edges VALUES('syn','target','calls',2,3,?1,NULL),('ra','target','function_ref',4,5,?2,NULL),('missing','target','calls',6,7,?3,NULL)",
        rusqlite::params![
            format!(r#"{{"resolvedBy":"{SYN}","refName":"call_syn","refKind":"calls"}}"#),
            format!(r#"{{"resolvedBy":"{RA}","refName":"call_ra","refKind":"function_ref"}}"#),
            format!(r#"{{"resolvedBy":"{RA}","refName":"orphan","refKind":"calls"}}"#),
        ],
    )
    .unwrap();
    // Unrelated graph rows make the VM-step assertion represent a realistic
    // full-table scan instead of a tiny fixture where the savings disappear.
    for i in 0..1_000 {
        db.execute(
            "INSERT INTO nodes VALUES(?1,?2,'rust')",
            rusqlite::params![format!("n{i}"), format!("src/{i}.rs")],
        )
        .unwrap();
        db.execute(
            "INSERT INTO edges VALUES(?1,'target','calls',1,1,'{\"resolvedBy\":\"other\"}',NULL)",
            [format!("n{i}")],
        )
        .unwrap();
    }
    db
}

fn old_select_vm_steps(db: &Connection) -> i32 {
    [SYN, RA]
        .into_iter()
        .map(|engine| {
            let mut statement = db.prepare(
                "SELECT e.source,json_extract(e.metadata,'$.refName'),json_extract(e.metadata,'$.refKind'),e.line,e.col,s.file_path,s.language FROM edges e JOIN nodes s ON s.id=e.source WHERE CASE WHEN json_valid(e.metadata) THEN json_extract(e.metadata,'$.resolvedBy') END=?1 LIMIT 20001",
            ).unwrap();
            let rows = statement
                .query_map([engine], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                    ))
                })
                .unwrap();
            for row in rows {
                std::hint::black_box(row.unwrap());
            }
            statement.get_status(StatementStatus::VmStep)
        })
        .sum()
}

fn combined_select_vm_steps(db: &Connection) -> i32 {
    let mut statement = db.prepare(invalidation::RUST_SCAN_SQL).unwrap();
    let rows = statement
        .query_map(rusqlite::params![SYN, RA], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })
        .unwrap();
    for row in rows {
        std::hint::black_box(row.unwrap());
    }
    statement.get_status(StatementStatus::VmStep)
}

#[test]
fn rust_invalidation_rechecks_both_engines_and_removes_orphan_edges() {
    let db = fixture();
    let refs = invalidate_rust(&db).unwrap();
    assert_eq!(refs.len(), 2);
    let actual: Vec<(String, String, String)> = db
        .prepare(
            "SELECT reference_name,reference_kind,file_path FROM unresolved_refs ORDER BY reference_name",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        actual,
        [
            ("call_ra".into(), "function_ref".into(), "src/ra.rs".into()),
            ("call_syn".into(), "calls".into(), "src/syn.rs".into()),
        ]
    );
    let remaining: i64 = db
        .query_row(
            "SELECT count(*) FROM edges WHERE CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.resolvedBy') END IN (?1,?2)",
            rusqlite::params![SYN, RA],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0);
    let malformed: i64 = db
        .query_row(
            "SELECT count(*) FROM edges WHERE metadata='{malformed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(malformed, 1);
    assert!(invalidate_rust(&db).unwrap().is_empty());
}

#[test]
fn combined_rust_invalidation_uses_fewer_sqlite_vm_steps_than_two_scans() {
    let db = fixture();
    let repeated = old_select_vm_steps(&db);
    let combined = combined_select_vm_steps(&db);
    eprintln!("Rust invalidation SELECT VM steps: separate={repeated}, combined={combined}");
    assert!(repeated > 0);
    assert!(
        combined * 3 < repeated * 2,
        "combined scan should use less than two thirds of the VM steps: combined={combined}, repeated={repeated}"
    );
}

#[test]
fn rust_invalidation_budget_failure_happens_before_deleting_edges() {
    let db = fixture();
    db.execute("DELETE FROM edges", []).unwrap();
    for i in 0..=super::super::MAX_ROWS {
        db.execute(
            "INSERT INTO edges VALUES('syn','target','calls',1,1,?1,NULL)",
            [format!(
                r#"{{"resolvedBy":"{SYN}","refName":"ref{i}","refKind":"calls"}}"#
            )],
        )
        .unwrap();
    }
    // 20,001 matching rows exceed this engine's legacy cap. Validation must
    // finish before the single DELETE executes.
    assert!(matches!(invalidate_rust(&db), Err(("SYNC_LIMIT", _))));
    let count: i64 = db
        .query_row("SELECT count(*) FROM edges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, super::super::MAX_ROWS as i64 + 1);
}

#[test]
fn rust_invalidation_rejects_missing_reference_metadata_before_writing() {
    for field in ["refName", "refKind"] {
        let db = fixture();
        let mut metadata = serde_json::json!({
            "resolvedBy": RA, "refName": "call_ra", "refKind": "calls"
        });
        metadata.as_object_mut().unwrap().remove(field);
        db.execute(
            "UPDATE edges SET metadata=?1 WHERE source='ra'",
            [metadata.to_string()],
        )
        .unwrap();
        let before: i64 = db
            .query_row("SELECT count(*) FROM edges", [], |r| r.get(0))
            .unwrap();
        assert!(invalidate_rust(&db).is_err(), "missing {field}");
        assert_eq!(
            db.query_row("SELECT count(*) FROM edges", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM unresolved_refs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn rust_invalidation_insert_failure_rolls_back_both_engines() {
    let mut db = fixture();
    db.execute_batch(
        "CREATE TRIGGER reject_ra BEFORE INSERT ON unresolved_refs
         WHEN NEW.from_node_id='ra' BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
    )
    .unwrap();
    let before: i64 = db
        .query_row("SELECT count(*) FROM edges", [], |r| r.get(0))
        .unwrap();
    let transaction = db.transaction().unwrap();
    // The legacy row inserts first; failure on the current engine must not
    // leave either partially recreated refs or deleted resolved edges.
    assert!(invalidate_rust(&transaction).is_err());
    assert_eq!(
        transaction
            .query_row("SELECT count(*) FROM unresolved_refs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    transaction.rollback().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM edges", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM unresolved_refs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
