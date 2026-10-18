//! Multi-row insertion adapted from CodeGraph 1.6.2 QueryBuilder.runBatched.
//! Upstream: https://github.com/colbymchenry/codegraph, src/db/queries.ts (MIT).
//! License notice retained in vendor/codegraph-native/LICENSE.
//! Keep input order and the upstream's four reusable SQL shapes. The caller
//! owns the transaction: a failed chunk must roll back the entire file.
use super::{INDEX, sql};
use harness_domain::{Failure, execution::OperationContext};
use rusqlite::{Connection, params_from_iter, types::Value};

pub(super) fn values(parameters: &[&dyn rusqlite::ToSql]) -> Result<Vec<Value>, Failure> {
    parameters
        .iter()
        .map(|parameter| match sql(parameter.to_sql())? {
            rusqlite::types::ToSqlOutput::Borrowed(value) => Ok(match value {
                rusqlite::types::ValueRef::Null => Value::Null,
                rusqlite::types::ValueRef::Integer(value) => Value::Integer(value),
                rusqlite::types::ValueRef::Real(value) => Value::Real(value),
                rusqlite::types::ValueRef::Text(value) => {
                    Value::Text(std::str::from_utf8(value).map_err(|_| INDEX)?.to_owned())
                }
                rusqlite::types::ValueRef::Blob(value) => Value::Blob(value.to_vec()),
            }),
            rusqlite::types::ToSqlOutput::Owned(value) => Ok(value),
            _ => Err(INDEX),
        })
        .collect()
}

pub(super) fn insert<T>(
    db: &Connection,
    head: &str,
    columns: usize,
    rows: &[T],
    context: &OperationContext,
    encode: impl Fn(&T) -> Result<Vec<Value>, Failure>,
) -> Result<(), Failure> {
    run(db, head, columns, rows, context, encode, false).map(|_| ())
}

pub(super) fn insert_ids<T>(
    db: &Connection,
    head: &str,
    columns: usize,
    rows: &[T],
    context: &OperationContext,
    encode: impl Fn(&T) -> Result<Vec<Value>, Failure>,
) -> Result<Vec<i64>, Failure> {
    run(db, head, columns, rows, context, encode, true)
}

#[allow(clippy::too_many_arguments)] // Shared ordered batch engine; RETURNING is explicit.
fn run<T>(
    db: &Connection,
    head: &str,
    columns: usize,
    rows: &[T],
    context: &OperationContext,
    encode: impl Fn(&T) -> Result<Vec<Value>, Failure>,
    returning: bool,
) -> Result<Vec<i64>, Failure> {
    let tuple = format!("({})", vec!["?"; columns].join(","));
    let mut offset = 0;
    let mut ids = Vec::new();
    for size in [128, 32, 8, 1] {
        if rows.len() - offset < size {
            continue;
        }
        let query = format!(
            "{head}{}{}",
            vec![tuple.as_str(); size].join(","),
            if returning { " RETURNING id" } else { "" }
        );
        let mut statement = sql(db.prepare_cached(&query))?;
        while rows.len() - offset >= size {
            let mut parameters = Vec::with_capacity(size * columns);
            for row in &rows[offset..offset + size] {
                context.check()?;
                let values = encode(row)?;
                if values.len() != columns {
                    return Err(INDEX);
                }
                parameters.extend(values);
            }
            if returning {
                let returned = sql(statement
                    .query_map(params_from_iter(parameters.iter()), |row| {
                        row.get::<_, i64>(0)
                    }))?;
                for id in returned {
                    ids.push(sql(id)?);
                }
            } else {
                sql(statement.execute(params_from_iter(parameters.iter())))?;
            }
            offset += size;
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_schema_write_phase_diagnostic() {
        const COUNT: usize = 1257; // Exercises 128, 32, 8 and 1-row shapes.
        const NODES: &str = "INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,docstring,signature,visibility,is_exported,is_async,is_static,is_abstract,decorators,type_parameters,return_type,updated_at) VALUES ";
        const EDGES: &str =
            "INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES ";
        const REFS: &str = "INSERT INTO unresolved_refs(from_node_id,reference_name,reference_kind,line,col,candidates,file_path,language) VALUES ";
        let nodes = (0..COUNT)
            .map(|i| {
                vec![
                    Value::Text(format!("n{i}")),
                    Value::Text("function".into()),
                    Value::Text(format!("symbol{i}")),
                    Value::Text(format!("symbol{i}")),
                    Value::Text("fixture.rs".into()),
                    Value::Text("rust".into()),
                    Value::Integer(i as i64 + 1),
                    Value::Integer(i as i64 + 1),
                    Value::Integer(0),
                    Value::Integer(1),
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Integer(0),
                    Value::Integer(0),
                    Value::Integer(0),
                    Value::Integer(0),
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Integer(0),
                ]
            })
            .collect::<Vec<_>>();
        let edges = (0..COUNT)
            .map(|i| {
                vec![
                    Value::Text("n0".into()),
                    Value::Text(format!("n{i}")),
                    Value::Text("calls".into()),
                    Value::Integer(i as i64 + 1),
                    Value::Integer(0),
                    Value::Null,
                    Value::Text("heuristic".into()),
                ]
            })
            .collect::<Vec<_>>();
        let refs = (0..COUNT)
            .map(|i| {
                vec![
                    Value::Text(format!("n{i}")),
                    Value::Text(format!("missing{i}")),
                    Value::Text("calls".into()),
                    Value::Integer(i as i64 + 1),
                    Value::Integer(0),
                    Value::Null,
                    Value::Text("fixture.rs".into()),
                    Value::Text("rust".into()),
                ]
            })
            .collect::<Vec<_>>();
        let context = OperationContext::default();
        for round in 0..3 {
            let modes = if round % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            };
            let mut counts = Vec::new();
            for batched in modes {
                let root = tempfile::tempdir().unwrap();
                let mut db = Connection::open(root.path().join("graph.db")).unwrap();
                db.execute_batch(include_str!("../../vendor/codegraph-native/schema.sql"))
                    .unwrap();
                db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-4096;").unwrap();
                let tx = db.transaction().unwrap();
                let start = std::time::Instant::now();
                let write = |head: &str, columns: usize, rows: &[Vec<Value>]| {
                    if batched {
                        insert(&tx, head, columns, rows, &context, |row| Ok(row.clone())).unwrap();
                    } else {
                        let tuple = format!("({})", vec!["?"; columns].join(","));
                        let mut statement = tx.prepare_cached(&format!("{head}{tuple}")).unwrap();
                        for row in rows {
                            context.check().unwrap();
                            let row = row.clone(); // Same owned parameter materialization in both paths.
                            statement.execute(params_from_iter(row.iter())).unwrap();
                        }
                    }
                };
                write(NODES, 21, &nodes);
                let node_time = start.elapsed();
                let start = std::time::Instant::now();
                write(EDGES, 7, &edges);
                let edge_time = start.elapsed();
                let start = std::time::Instant::now();
                let ids = if batched {
                    insert_ids(&tx, REFS, 8, &refs, &context, |row| Ok(row.clone())).unwrap()
                } else {
                    let mut statement = tx
                        .prepare_cached(&format!("{REFS}(?,?,?,?,?,?,?,?)"))
                        .unwrap();
                    let mut ids = Vec::new();
                    for row in &refs {
                        context.check().unwrap();
                        let row = row.clone();
                        statement.execute(params_from_iter(row.iter())).unwrap();
                        ids.push(tx.last_insert_rowid());
                    }
                    ids
                };
                let ref_time = start.elapsed();
                assert_eq!(ids.len(), COUNT);
                assert_eq!(
                    ids.iter()
                        .copied()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len(),
                    COUNT
                );
                let start = std::time::Instant::now();
                assert!(
                    !tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check LIMIT 1)",
                        [],
                        |r| r.get::<_, bool>(0)
                    )
                    .unwrap()
                );
                let integrity_time = start.elapsed();
                let start = std::time::Instant::now();
                tx.commit().unwrap();
                let commit_time = start.elapsed();
                let observed: (i64,i64,i64,i64) = db.query_row("SELECT (SELECT count(*) FROM nodes),(SELECT count(*) FROM edges),(SELECT count(*) FROM unresolved_refs),(SELECT count(*) FROM nodes_fts WHERE nodes_fts MATCH 'symbol0')", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
                assert_eq!(observed, (COUNT as i64, COUNT as i64, COUNT as i64, 1));
                counts.push(observed);
                println!(
                    "{}",
                    serde_json::json!({"component":"real-schema-write-phases","round":round+1,"mode":if batched {"batches"}else{"prepared-per-row"},"rowsPerTable":COUNT,"nodesMs":node_time.as_secs_f64()*1000.0,"edgesMs":edge_time.as_secs_f64()*1000.0,"refsMs":ref_time.as_secs_f64()*1000.0,"integrityMs":integrity_time.as_secs_f64()*1000.0,"commitMs":commit_time.as_secs_f64()*1000.0,"ftsAndForeignKeysVerified":true,"journal":"wal","synchronous":"full","debugBuild":cfg!(debug_assertions),"schemaCreationAndRowsPreparationIncluded":false,"wholeIndexSpeedupClaimed":false})
                );
            }
            assert_eq!(counts[0], counts[1]);
        }
    }

    #[test]
    fn returning_collects_exact_inserted_ids_across_batches() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE sample(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO sample(id,name) VALUES(9000,'prior')").unwrap();
        let tx = db.transaction().unwrap();
        let rows = (0..300_i64).collect::<Vec<_>>();
        let ids = insert_ids(
            &tx,
            "INSERT INTO sample(name) VALUES ",
            1,
            &rows,
            &OperationContext::default(),
            |i| Ok(vec![Value::Text(i.to_string())]),
        )
        .unwrap();
        assert_eq!(ids.len(), 300);
        let actual: std::collections::BTreeSet<i64> = tx
            .prepare("SELECT id FROM sample WHERE name!='prior'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            ids.into_iter().collect::<std::collections::BTreeSet<_>>(),
            actual
        );
        tx.commit().unwrap();
    }

    #[test]
    fn batch_boundaries_preserve_rowids_nulls_and_input_order() {
        for count in [0, 1, 7, 8, 31, 32, 127, 128, 169, 300] {
            let mut db = Connection::open_in_memory().unwrap();
            db.execute_batch("CREATE TABLE sample(name TEXT, nullable TEXT)")
                .unwrap();
            let tx = db.transaction().unwrap();
            let rows = (0..count).collect::<Vec<_>>();
            insert(
                &tx,
                "INSERT INTO sample(name,nullable) VALUES ",
                2,
                &rows,
                &OperationContext::default(),
                |i| Ok(vec![Value::Text(i.to_string()), Value::Null]),
            )
            .unwrap();
            tx.commit().unwrap();
            let mut stmt = db
                .prepare("SELECT rowid,name,nullable FROM sample ORDER BY rowid")
                .unwrap();
            let actual = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(actual.len(), count);
            for (i, (rowid, name, nullable)) in actual.into_iter().enumerate() {
                assert_eq!(rowid, i as i64 + 1);
                assert_eq!(name, i.to_string());
                assert_eq!(nullable, None);
            }
        }
    }

    #[test]
    fn late_chunk_constraint_failure_rolls_back_earlier_chunks() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE sample(id INTEGER PRIMARY KEY)")
            .unwrap();
        {
            let tx = db.transaction().unwrap();
            let mut rows = (0..169).collect::<Vec<i64>>();
            rows[168] = 0;
            assert!(
                insert(
                    &tx,
                    "INSERT INTO sample(id) VALUES ",
                    1,
                    &rows,
                    &OperationContext::default(),
                    |id| Ok(vec![Value::Integer(*id)])
                )
                .is_err()
            );
        }
        assert_eq!(
            db.query_row("SELECT count(*) FROM sample", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn conflict_ignore_keeps_first_row_and_original_order() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE sample(id INTEGER UNIQUE, name TEXT)")
            .unwrap();
        let rows = (0..169_i64).collect::<Vec<_>>();
        insert(
            &db,
            "INSERT OR IGNORE INTO sample(id,name) VALUES ",
            2,
            &rows,
            &OperationContext::default(),
            |i| Ok(vec![Value::Integer(i % 17), Value::Text(i.to_string())]),
        )
        .unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM sample", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            17
        );
        assert_eq!(
            db.query_row("SELECT name FROM sample WHERE id=0", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "0"
        );
    }
}
