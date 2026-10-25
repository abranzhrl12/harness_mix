//! Retire incompatible derived edges without blocking source publication.
//! The archive is transactional, deliberately has no node FKs, and is never
//! traversed as active graph evidence. Only a provider can regenerate it.
use super::super::{LIMIT, MAX_ROWS, sql};
use harness_domain::{Failure, execution::OperationContext, hash};
use rusqlite::{Connection, Transaction, params};
use serde_json::{Value, json};

pub(super) fn invalidate(
    db: &Transaction<'_>,
    path: &str,
    context: &OperationContext,
) -> Result<(), Failure> {
    let mut statement = sql(db.prepare(
        "SELECT e.id,json_object('source',e.source,'target',e.target,'kind',e.kind,
         'line',e.line,'col',e.col,'metadata',e.metadata,'provenance',e.provenance,
         'sourcePath',s.file_path,'targetPath',t.file_path,
         'sourceHash',sf.content_hash,'targetHash',tf.content_hash)
         FROM edges e LEFT JOIN nodes s ON s.id=e.source LEFT JOIN nodes t ON t.id=e.target
         LEFT JOIN files sf ON sf.path=s.file_path LEFT JOIN files tf ON tf.path=t.file_path
         WHERE CASE WHEN json_valid(e.metadata) THEN json_extract(e.metadata,'$.synthesizedBy') IS NOT NULL ELSE 0 END
         AND ((s.file_path=?1 AND (s.kind NOT IN('route','component') OR s.id LIKE 'native:framework:%'))
           OR (t.file_path=?1 AND (t.kind NOT IN('route','component') OR t.id LIKE 'native:framework:%'))
           OR (CASE WHEN json_valid(e.metadata) THEN json_extract(e.metadata,'$.registeredAt') END >= ?2
           AND CASE WHEN json_valid(e.metadata) THEN json_extract(e.metadata,'$.registeredAt') END < ?3))
         ORDER BY e.id LIMIT 20001",
    ))?;
    let rows = sql(statement
        .query_map(params![path, format!("{path}:"), format!("{path};")], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        }))?;
    let mut snapshots = Vec::new();
    let mut bytes = 0usize;
    for row in rows {
        context.check()?;
        let row = sql(row)?;
        bytes += row.1.len();
        if snapshots.len() >= MAX_ROWS || bytes > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        snapshots.push(row);
    }
    drop(statement);
    if snapshots.is_empty() {
        return Ok(());
    }
    sql(db.execute_batch(
        "CREATE TABLE IF NOT EXISTS native_invalidated_edges(
         origin_path TEXT NOT NULL, snapshot_hash TEXT NOT NULL, snapshot TEXT NOT NULL,
         reason TEXT NOT NULL, PRIMARY KEY(origin_path,snapshot_hash))",
    ))?;
    for (id, snapshot) in snapshots {
        context.check()?;
        sql(db.execute(
            "INSERT OR IGNORE INTO native_invalidated_edges VALUES(?1,?2,?3,'provider_synthesis_requires_regeneration')",
            params![path, hash(snapshot.as_bytes()), snapshot],
        ))?;
        sql(db.execute("DELETE FROM edges WHERE id=?1", [id]))?;
    }
    Ok(())
}

pub(crate) fn count(db: &Connection, path: Option<&str>) -> Result<usize, Failure> {
    let exists: bool = sql(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='native_invalidated_edges')",
        [], |r| r.get(0),
    ))?;
    if !exists {
        return Ok(0);
    }
    let count: i64 = sql(db.query_row(
        "SELECT count(*) FROM native_invalidated_edges WHERE ?1 IS NULL OR origin_path=?1",
        [path],
        |r| r.get(0),
    ))?;
    usize::try_from(count).map_err(|_| LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_edge_invalidation_is_transactional_and_preserves_unrelated_edges() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE nodes(id TEXT PRIMARY KEY,file_path TEXT,kind TEXT);
            CREATE TABLE files(path TEXT PRIMARY KEY,content_hash TEXT);
            CREATE TABLE edges(id INTEGER PRIMARY KEY,source TEXT,target TEXT,kind TEXT,line INTEGER,col INTEGER,metadata TEXT,provenance TEXT);
            INSERT INTO nodes VALUES('a','a.rs','function'),('b','b.rs','function'),('c','c.rs','function');
            INSERT INTO files VALUES('a.rs','hash-a'),('b.rs','hash-b');
            INSERT INTO edges VALUES(1,'a','b','calls',1,0,'{\"synthesizedBy\":\"framework\"}','heuristic'),
                (2,'b','c','calls',2,0,'{\"synthesizedBy\":\"framework\"}',NULL),
                (3,'a','b','references',3,0,NULL,NULL);").unwrap();
        {
            let tx = db.transaction().unwrap();
            invalidate(&tx, "a.rs", &OperationContext::default()).unwrap();
            assert_eq!(count(&tx, Some("a.rs")).unwrap(), 1);
            // A failure after capture must roll back archive creation/deletion,
            // not just fail schema validation before the publication starts.
            assert!(
                tx.execute("INSERT INTO nodes VALUES('a','other.rs','function')", [])
                    .is_err()
            );
            tx.rollback().unwrap();
        }
        assert_eq!(count(&db, None).unwrap(), 0);
        assert_eq!(
            db.query_row("SELECT count(*) FROM edges", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        let tx = db.transaction().unwrap();
        invalidate(&tx, "a.rs", &OperationContext::default()).unwrap();
        tx.commit().unwrap();
        assert_eq!(count(&db, Some("a.rs")).unwrap(), 1);
        assert_eq!(count(&db, Some("b.rs")).unwrap(), 0);
        assert_eq!(
            db.query_row("SELECT count(*) FROM edges WHERE id IN (2,3)", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}

pub(crate) fn annotate(report: &mut Value, count: usize, exploration: bool) {
    if count == 0 {
        return;
    }
    let field = if exploration {
        "diagnostics"
    } else {
        "warnings"
    };
    if !report[field].is_array() {
        report[field] = json!([]);
    }
    let mut warning = json!({
        "code":"INDEX_DERIVED_RELATIONS_INVALIDATED",
        "message":"Incompatible synthesized relations were archived, not validated or regenerated; source indexing can continue without deleting the database.",
        "archivedRelations":count,
        "scope":if exploration { "workspace" } else { "file" }
    });
    if !exploration {
        warning["path"] = report["path"].clone();
    }
    report[field].as_array_mut().unwrap().push(warning);
    report["graphStatus"] = json!("partial");
    if exploration {
        report["_complete"] = json!(false);
        report["absenceProven"] = json!(false);
    } else {
        report["warningFiles"] = json!(report["warningFiles"].as_u64().unwrap_or(0).max(1));
    }
}
