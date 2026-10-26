//! Per-file native proof version. Source hashes alone cannot detect a resolver upgrade.
use super::{Failure, LIMIT, MAX_ROWS, sql};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeSet;

pub(super) const RUST_VERSION: &str = "rust-native-proof-v2";
const PREFIX: &str = "native_rust_proof:";

pub(super) fn current(db: &Connection, path: &str) -> Result<bool, Failure> {
    let stored: Option<String> = sql(db
        .query_row(
            "SELECT value FROM project_metadata WHERE key=?1",
            [format!("{PREFIX}{path}")],
            |row| row.get(0),
        )
        .optional())?;
    Ok(stored.as_deref() == Some(RUST_VERSION))
}

pub(super) fn record(db: &Connection, path: &str, present: bool, now: i64) -> Result<(), Failure> {
    if present {
        sql(db.execute(
            "INSERT INTO project_metadata(key,value,updated_at) VALUES(?1,?2,?3)
            ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
            params![format!("{PREFIX}{path}"), RUST_VERSION, now],
        ))?;
    } else {
        sql(db.execute(
            "DELETE FROM project_metadata WHERE key=?1",
            [format!("{PREFIX}{path}")],
        ))?;
    }
    Ok(())
}

pub(super) fn outdated(db: &Connection) -> Result<BTreeSet<String>, Failure> {
    let mut statement = sql(db.prepare(
        "SELECT f.path FROM files f LEFT JOIN project_metadata m
        ON m.key=?1 || f.path WHERE f.path LIKE '%.rs' AND coalesce(m.value,'')!=?2
        ORDER BY f.path LIMIT 20001",
    ))?;
    let rows = sql(statement.query_map(params![PREFIX, RUST_VERSION], |r| r.get::<_, String>(0)))?;
    let mut paths = BTreeSet::new();
    for row in rows {
        if paths.len() >= MAX_ROWS {
            return Err(LIMIT);
        }
        paths.insert(sql(row)?);
    }
    Ok(paths)
}
