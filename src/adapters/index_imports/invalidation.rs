//! Invalidation helpers for engine-owned resolved edges.
use super::{Failure, INDEX, LIMIT, sql};
use rusqlite::Connection;

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
