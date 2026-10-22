//! External module changes affect named bindings, not every call in an importer.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn catalog(db: &Connection) -> Result<Option<String>, Failure> {
    if std::env::var("HARNESS_INDEX_RETRY_BINDINGS").as_deref() == Ok("0") {
        return Ok(None);
    }
    // This optional selection aid must not reject a publication that the
    // authoritative resolver could perform. Unknown coverage uses its old gate.
    Ok(catalog_inner(db).unwrap_or(None))
}

fn checked<T>(value: rusqlite::Result<T>) -> Result<T, Failure> {
    value.map_err(|_| INDEX)
}

fn catalog_inner(db: &Connection) -> Result<Option<String>, Failure> {
    let mut statement = checked(db.prepare_cached("SELECT file_path,signature FROM nodes WHERE kind='import' ORDER BY file_path,start_line,id LIMIT 20001"))?;
    let rows = checked(statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    }))?;
    let mut pairs = BTreeSet::<(String, String)>::new();
    let mut budgets = BTreeMap::<String, (usize, usize)>::new();
    let mut bytes = 0usize;
    for (index, row) in rows.enumerate() {
        let (path, signature) = checked(row)?;
        if index >= 20_000 || bytes > 2 * 1024 * 1024 {
            return Ok(None);
        }
        let budget = budgets.entry(path.clone()).or_default();
        budget.0 += 1;
        if let Some(signature) = signature {
            budget.1 += signature.len();
            bytes += signature.len();
            if budget.0 > 256 || budget.1 > 1024 * 1024 {
                return Ok(None);
            }
            if let Some((_, names)) = codegraph_native::named_imports(&signature) {
                pairs.extend(names.into_iter().map(|(_, local)| (path.clone(), local)));
            }
        }
        if pairs.len() > 4096 {
            return Ok(None);
        }
    }
    // Keep actual local names too: global homonyms can affect the existing
    // candidate ceiling even when the reference has no named-import binding.
    let mut statement = checked(db.prepare_cached("SELECT file_path,name,qualified_name FROM nodes WHERE language IN ('typescript','tsx','javascript','jsx') AND file_path IN(SELECT file_path FROM nodes WHERE kind='import') AND kind IN ('function','method','class','struct','interface','trait','record','enum') LIMIT 20001"))?;
    let rows = checked(statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    }))?;
    for (index, row) in rows.enumerate() {
        let (path, name, qualified) = checked(row)?;
        if index >= 20_000 || pairs.len() > 4096 {
            return Ok(None);
        }
        pairs.insert((path.clone(), name));
        pairs.insert((path, qualified));
    }
    let bytes: usize = pairs
        .iter()
        .map(|(path, name)| path.len() + name.len() + 8)
        .sum();
    if bytes > 65536 {
        return Ok(None);
    }
    Ok(Some(serde_json::to_string(&pairs).map_err(|_| INDEX)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_keeps_import_aliases_and_local_candidate_names_but_not_arbitrary_calls() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE nodes(file_path TEXT,kind TEXT,name TEXT,qualified_name TEXT,signature TEXT,start_line INTEGER,id TEXT,language TEXT);").unwrap();
        db.execute(
            "INSERT INTO nodes VALUES('app.js','import','./dep','./dep',?1,1,'i','javascript')",
            ["import { run as imported } from './dep';"],
        )
        .unwrap();
        db.execute(
            "INSERT INTO nodes VALUES('app.js','function','local','scope.local',NULL,2,'f','javascript')",
            [],
        )
        .unwrap();
        let text = catalog_inner(&db).unwrap().unwrap();
        let pairs: BTreeSet<(String, String)> = serde_json::from_str(&text).unwrap();
        assert_eq!(
            pairs,
            BTreeSet::from([
                ("app.js".into(), "imported".into()),
                ("app.js".into(), "local".into()),
                ("app.js".into(), "scope.local".into())
            ])
        );
    }
}
