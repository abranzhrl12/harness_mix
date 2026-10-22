//! Invalidation helpers for engine-owned resolved edges.
use super::{Failure, INDEX, LIMIT, sql};
use rusqlite::Connection;

type Original = (String, String, String, i64, i64, String, String);

/// One snapshot scan/delete for the requested engines. Preserve each engine's
/// independent bounds and insertion order; malformed metadata and orphan nodes
/// have exactly the same treatment as the individual invalidations.
pub(super) fn engines(db: &Connection, engines: &[&str]) -> Result<Vec<Vec<i64>>, Failure> {
    if engines.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", engines.len())
        .collect::<Vec<_>>()
        .join(",");
    let predicate = format!(
        "CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.resolvedBy') END IN ({placeholders})"
    );
    let query = format!(
        "SELECT e.source,json_extract(e.metadata,'$.refName'),json_extract(e.metadata,'$.refKind'),e.line,e.col,s.file_path,s.language,json_extract(e.metadata,'$.resolvedBy') FROM edges e JOIN nodes s ON s.id=e.source WHERE CASE WHEN json_valid(e.metadata) THEN json_extract(e.metadata,'$.resolvedBy') END IN ({placeholders}) LIMIT {}",
        engines.len() * (super::super::MAX_ROWS + 1)
    );
    let mut statement = sql(db.prepare(&query))?;
    let rows = sql(
        statement.query_map(rusqlite::params_from_iter(engines), |row| {
            Ok((
                (
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ),
                row.get::<_, String>(7)?,
            ))
        }),
    )?;
    let mut grouped: Vec<Vec<Original>> = vec![Vec::new(); engines.len()];
    let mut reserved = vec![0usize; engines.len()];
    for row in rows {
        let (original, engine) = sql(row)?;
        let index = engines
            .iter()
            .position(|expected| *expected == engine)
            .ok_or(INDEX)?;
        reserved[index] += original.0.len()
            + original.1.len()
            + original.2.len()
            + original.5.len()
            + original.6.len();
        if grouped[index].len() >= super::super::MAX_ROWS || reserved[index] > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        grouped[index].push(original);
    }
    drop(statement);
    sql(db.execute(
        &format!("DELETE FROM edges WHERE {predicate}"),
        rusqlite::params_from_iter(engines),
    ))?;
    let mut result = Vec::with_capacity(engines.len());
    for rows in grouped {
        let mut ids = Vec::with_capacity(rows.len());
        for (source, name, kind, line, col, path, language) in rows {
            ids.push(super::super::insert_ref(
                db,
                &source,
                &name,
                &kind,
                line,
                col,
                None,
                (&path, &language),
            )?);
        }
        result.push(ids);
    }
    Ok(result)
}

const SYN: &str = "syn-module-subset-v1";
const RA: &str = "ra-module-subset-v1";
pub(super) const RUST_SCAN_SQL: &str = "SELECT e.source,json_extract(e.metadata,'$.refName'),json_extract(e.metadata,'$.refKind'),e.line,e.col,s.file_path,s.language,CASE WHEN json_valid(e.metadata) THEN json_extract(e.metadata,'$.resolvedBy') END FROM edges e JOIN nodes s ON s.id=e.source WHERE CASE WHEN json_valid(e.metadata) THEN json_extract(e.metadata,'$.resolvedBy') END IN (?1,?2) LIMIT 40002";

/// Invalidate both Rust import engines with one metadata scan and one delete.
/// Rust publication replaces both engines together, so scanning the edge table
/// once avoids repeating the same JSON extraction and table walk. Keep the
/// original per-engine budgets and result order; malformed metadata remains
/// excluded from rechecks, while matching orphan edges are still deleted.
pub(super) fn rust(db: &Connection) -> Result<Vec<i64>, Failure> {
    let mut statement = sql(db.prepare(RUST_SCAN_SQL))?;
    let rows = sql(statement.query_map(rusqlite::params![SYN, RA], |r| {
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
    }))?;
    let mut syn_rows = Vec::new();
    let mut ra_rows = Vec::new();
    let mut syn_reserved = 0usize;
    let mut ra_reserved = 0usize;
    for row in rows {
        let (source, name, kind, line, col, path, language, engine) = sql(row)?;
        let (engine_rows, engine_reserved) = match engine.as_str() {
            SYN => (&mut syn_rows, &mut syn_reserved),
            RA => (&mut ra_rows, &mut ra_reserved),
            _ => return Err(INDEX),
        };
        *engine_reserved += source.len() + name.len() + kind.len() + path.len() + language.len();
        if engine_rows.len() >= super::super::MAX_ROWS || *engine_reserved > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        engine_rows.push((source, name, kind, line, col, path, language));
    }

    sql(db.execute(
        "DELETE FROM edges WHERE CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.resolvedBy') END IN (?1,?2)",
        rusqlite::params![SYN, RA],
    ))?;
    let mut refs = Vec::new();
    for (source, name, kind, line, col, path, language) in syn_rows.into_iter().chain(ra_rows) {
        refs.push(super::super::insert_ref(
            db,
            &source,
            &name,
            &kind,
            line,
            col,
            None,
            (&path, &language),
        )?);
    }
    Ok(refs)
}
