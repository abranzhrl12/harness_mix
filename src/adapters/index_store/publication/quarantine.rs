//! Retire incompatible derived edges without blocking source publication.
//! The archive is transactional, deliberately has no node FKs, and is never
//! traversed as active graph evidence. Coverage is derived from current endpoints
//! and typed replacements, not from the size of this historical archive.
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
         'sourceQualifiedName',s.qualified_name,'targetQualifiedName',t.qualified_name,
         'sourceKind',s.kind,'targetKind',t.kind,
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

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Coverage {
    pub(crate) pending: usize,
    pub(crate) superseded: usize,
    pub(crate) obsolete: usize,
}

/// Assess only archive entries relevant to a file or a returned graph. No schema
/// writes on reads: old archives need no migration to gain current-state status.
pub(crate) fn coverage(
    db: &Connection,
    path: Option<&str>,
    nodes: Option<&std::collections::BTreeSet<String>>,
) -> Result<Coverage, Failure> {
    let exists: bool = sql(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='native_invalidated_edges')",
        [], |r| r.get(0),
    ))?;
    if !exists {
        return Ok(Coverage::default());
    }
    let node_ids = nodes.map(|nodes| json!(nodes).to_string());
    let (pending, superseded, obsolete): (i64, i64, i64) = sql(db.query_row(
        "WITH archive AS (
           SELECT origin_path, CASE WHEN json_valid(snapshot) THEN snapshot ELSE '{}' END AS data
           FROM native_invalidated_edges
         ), scoped AS (
           SELECT json_extract(data,'$.source') AS source, json_extract(data,'$.target') AS target,
             json_extract(data,'$.sourcePath') AS source_path, json_extract(data,'$.targetPath') AS target_path,
             json_extract(data,'$.sourceQualifiedName') AS source_name, json_extract(data,'$.targetQualifiedName') AS target_name,
             json_extract(data,'$.sourceKind') AS source_kind, json_extract(data,'$.targetKind') AS target_kind,
             CASE WHEN json_extract(data,'$.metadata') IS NULL THEN json_extract(data,'$.kind')
               WHEN json_valid(json_extract(data,'$.metadata')) THEN
               CASE WHEN json_extract(json_extract(data,'$.metadata'),'$.synthesizedBy')='interface-impl'
                 THEN 'implements' ELSE json_extract(data,'$.kind') END
               ELSE NULL END AS expected_kind
           FROM archive WHERE (?1 IS NULL AND ?2 IS NULL)
             OR (?1 IS NOT NULL AND (origin_path=?1 OR json_extract(data,'$.sourcePath')=?1 OR json_extract(data,'$.targetPath')=?1))
             OR (?2 IS NOT NULL AND (json_extract(data,'$.source') IN(SELECT value FROM json_each(?2))
               OR json_extract(data,'$.target') IN(SELECT value FROM json_each(?2))))
         ), assessed AS (
           SELECT CASE
             WHEN source IS NULL OR target IS NULL OR expected_kind IS NULL THEN 'pending'
             WHEN NOT EXISTS(SELECT 1 FROM nodes WHERE id=scoped.source)
               AND source_name IS NOT NULL AND source_kind IS NOT NULL
               AND NOT EXISTS(SELECT 1 FROM nodes WHERE file_path=source_path AND qualified_name=source_name AND kind=source_kind) THEN 'obsolete'
             WHEN NOT EXISTS(SELECT 1 FROM nodes WHERE id=scoped.target)
               AND target_name IS NOT NULL AND target_kind IS NOT NULL
               AND NOT EXISTS(SELECT 1 FROM nodes WHERE file_path=target_path AND qualified_name=target_name AND kind=target_kind) THEN 'obsolete'
             WHEN NOT EXISTS(SELECT 1 FROM nodes WHERE id=scoped.source)
               OR NOT EXISTS(SELECT 1 FROM nodes WHERE id=scoped.target) THEN 'pending'
             WHEN EXISTS(SELECT 1 FROM edges e WHERE e.source=scoped.source AND e.target=scoped.target AND e.kind=scoped.expected_kind
               AND CASE WHEN e.metadata IS NULL THEN 1 WHEN json_valid(e.metadata) THEN json_extract(e.metadata,'$.synthesizedBy') IS NULL ELSE 0 END) THEN 'superseded'
             ELSE 'pending' END AS status FROM scoped
         ) SELECT coalesce(sum(status='pending'),0),coalesce(sum(status='superseded'),0),coalesce(sum(status='obsolete'),0) FROM assessed",
        params![path, node_ids],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ))?;
    Ok(Coverage {
        pending: usize::try_from(pending).map_err(|_| LIMIT)?,
        superseded: usize::try_from(superseded).map_err(|_| LIMIT)?,
        obsolete: usize::try_from(obsolete).map_err(|_| LIMIT)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_status_tracks_typed_replacements_and_scope_without_writing() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE nodes(id TEXT PRIMARY KEY,file_path TEXT,qualified_name TEXT,kind TEXT);
            CREATE TABLE edges(source TEXT,target TEXT,kind TEXT,metadata TEXT);
            CREATE TABLE native_invalidated_edges(origin_path TEXT,snapshot_hash TEXT,snapshot TEXT,reason TEXT);
            INSERT INTO nodes VALUES('trait','port.rs','Port::run','method'),('implementation','impl.rs','Impl::run','method'),('unrelated','other.rs','unrelated','function');").unwrap();
        let snapshot = json!({"source":"trait","target":"implementation","kind":"calls",
            "sourcePath":"port.rs","targetPath":"impl.rs","sourceQualifiedName":"Port::run","targetQualifiedName":"Impl::run","sourceKind":"method","targetKind":"method",
            "metadata":json!({"synthesizedBy":"interface-impl"}).to_string()});
        db.execute(
            "INSERT INTO native_invalidated_edges VALUES('port.rs','snapshot',?1,'legacy')",
            [snapshot.to_string()],
        )
        .unwrap();
        db.execute_batch("PRAGMA query_only=ON").unwrap();
        assert_eq!(coverage(&db, None, None).unwrap().pending, 1);
        assert_eq!(
            coverage(&db, Some("unrelated.rs"), None).unwrap(),
            Coverage::default()
        );
        assert_eq!(
            coverage(
                &db,
                None,
                Some(&std::collections::BTreeSet::from(["unrelated".to_owned()]))
            )
            .unwrap(),
            Coverage::default()
        );
        db.execute_batch("PRAGMA query_only=OFF; INSERT INTO edges VALUES('trait','implementation','calls',NULL)").unwrap();
        // A call edge is not proof of a trait implementation.
        assert_eq!(coverage(&db, None, None).unwrap().pending, 1);
        db.execute_batch("INSERT INTO edges VALUES('trait','implementation','implements',NULL)")
            .unwrap();
        assert_eq!(
            coverage(&db, Some("impl.rs"), None).unwrap(),
            Coverage {
                superseded: 1,
                ..Coverage::default()
            }
        );
        db.execute_batch("DELETE FROM nodes WHERE id='implementation'")
            .unwrap();
        assert_eq!(
            coverage(&db, None, None).unwrap(),
            Coverage {
                obsolete: 1,
                ..Coverage::default()
            }
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM native_invalidated_edges", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            1
        );
    }

    #[test]
    fn corrupt_archive_metadata_is_unknown_not_an_error_or_success() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE nodes(id TEXT,file_path TEXT,qualified_name TEXT,kind TEXT); CREATE TABLE edges(source TEXT,target TEXT,kind TEXT,metadata TEXT);
            CREATE TABLE native_invalidated_edges(origin_path TEXT,snapshot_hash TEXT,snapshot TEXT,reason TEXT);
            INSERT INTO native_invalidated_edges VALUES('a.rs','one','invalid JSON','legacy');").unwrap();
        assert_eq!(coverage(&db, None, None).unwrap().pending, 1);
        db.execute_batch("INSERT INTO nodes VALUES('a','a.rs','a','function'),('b','b.rs','b','function'); INSERT INTO edges VALUES('a','b','calls',NULL)").unwrap();
        db.execute(
            "INSERT INTO native_invalidated_edges VALUES('a.rs','two',?1,'legacy')",
            [
                json!({"source":"a","target":"b","kind":"calls","metadata":"invalid JSON"})
                    .to_string(),
            ],
        )
        .unwrap();
        assert_eq!(coverage(&db, None, None).unwrap().pending, 2);
    }

    #[test]
    fn derived_edge_invalidation_is_transactional_and_preserves_unrelated_edges() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE nodes(id TEXT PRIMARY KEY,file_path TEXT,kind TEXT,qualified_name TEXT);
            CREATE TABLE files(path TEXT PRIMARY KEY,content_hash TEXT);
            CREATE TABLE edges(id INTEGER PRIMARY KEY,source TEXT,target TEXT,kind TEXT,line INTEGER,col INTEGER,metadata TEXT,provenance TEXT);
            INSERT INTO nodes(id,file_path,kind) VALUES('a','a.rs','function'),('b','b.rs','function'),('c','c.rs','function');
            INSERT INTO files VALUES('a.rs','hash-a'),('b.rs','hash-b');
            INSERT INTO edges VALUES(1,'a','b','calls',1,0,'{\"synthesizedBy\":\"framework\"}','heuristic'),
                (2,'b','c','calls',2,0,'{\"synthesizedBy\":\"framework\"}',NULL),
                (3,'a','b','references',3,0,NULL,NULL);").unwrap();
        {
            let tx = db.transaction().unwrap();
            invalidate(&tx, "a.rs", &OperationContext::default()).unwrap();
            assert_eq!(coverage(&tx, Some("a.rs"), None).unwrap().pending, 1);
            // A failure after capture must roll back archive creation/deletion,
            // not just fail schema validation before the publication starts.
            assert!(
                tx.execute(
                    "INSERT INTO nodes(id,file_path,kind) VALUES('a','other.rs','function')",
                    []
                )
                .is_err()
            );
            tx.rollback().unwrap();
        }
        assert_eq!(coverage(&db, None, None).unwrap(), Coverage::default());
        assert_eq!(
            db.query_row("SELECT count(*) FROM edges", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        let tx = db.transaction().unwrap();
        invalidate(&tx, "a.rs", &OperationContext::default()).unwrap();
        tx.commit().unwrap();
        assert_eq!(coverage(&db, Some("a.rs"), None).unwrap().pending, 1);
        assert_eq!(coverage(&db, Some("c.rs"), None).unwrap().pending, 0);
        assert_eq!(
            db.query_row("SELECT count(*) FROM edges WHERE id IN (2,3)", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}

pub(crate) fn annotate(report: &mut Value, coverage: &Coverage, exploration: bool) {
    let total = coverage.pending + coverage.superseded + coverage.obsolete;
    if total == 0 {
        return;
    }
    report["indexArchive"] = json!({"pending":coverage.pending,"superseded":coverage.superseded,"obsolete":coverage.obsolete});
    if coverage.pending == 0 {
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
        "message":"Archived derived relations in this scope still lack a current typed replacement; archive history alone is not a graph defect.",
        "pendingRelations":coverage.pending,
        "scope":if exploration { "selected_graph" } else if report.get("path").is_some() { "file" } else { "workspace" }
    });
    if !exploration && report.get("path").is_some() {
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
