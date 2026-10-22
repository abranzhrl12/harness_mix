//! Cross-file retries only for declarations that the generic resolver can see.
//! Module/framework retries remain conservative: barrels and newly ambiguous
//! module paths can affect importers even without an existing incoming edge.
use super::{Failure, Transaction, sql};

/// Seek name/file indexes for selective retries instead of filtering every pending
/// reference through all name subqueries. The final branch keeps conservative
/// framework retries. CASE preserves the status predicate without making the
/// status-only index win over selective name/file seeks. No index is required.
pub(super) const PENDING_SQL: &str = "
WITH all_names(value) AS MATERIALIZED (
    SELECT name FROM nodes WHERE file_path=?1
    UNION SELECT qualified_name FROM nodes WHERE file_path=?1
), public_names(value) AS MATERIALIZED (
    SELECT name FROM nodes WHERE file_path=?1 AND (visibility='public' OR ifnull(is_exported,0)<>0)
        AND kind IN ('function','method','class','struct','interface','trait','record','enum')
    UNION SELECT qualified_name FROM nodes WHERE file_path=?1 AND (visibility='public' OR ifnull(is_exported,0)<>0)
        AND kind IN ('function','method','class','struct','interface','trait','record','enum')
), fresh_names(value) AS MATERIALIZED (
    SELECT value FROM all_names WHERE ?2 NOT IN ('typescript','tsx','javascript','jsx')
    UNION SELECT value FROM public_names WHERE ?2 IN ('typescript','tsx','javascript','jsx')
        AND value NOT IN (SELECT value FROM json_each(?4))
)
SELECT id FROM unresolved_refs
WHERE reference_name IN (SELECT value FROM fresh_names) AND CASE WHEN status='pending' THEN 1 ELSE 0 END
UNION
SELECT id FROM unresolved_refs
WHERE ?2 IN ('typescript','tsx','javascript','jsx') AND reference_name IN (SELECT value FROM public_names)
    AND language NOT IN ('typescript','tsx','javascript','jsx') AND CASE WHEN status='pending' THEN 1 ELSE 0 END
UNION
SELECT id FROM unresolved_refs
WHERE file_path=?1 AND reference_name IN (SELECT value FROM all_names)
    AND CASE WHEN status='pending' THEN 1 ELSE 0 END
UNION
SELECT id FROM unresolved_refs
WHERE (?2 IN ('typescript','tsx','javascript','jsx') OR ?3=1)
    AND language IN ('typescript','tsx','javascript','jsx')
    AND ?5 IS NULL AND file_path IN (SELECT file_path FROM nodes WHERE kind='import')
    AND CASE WHEN status='pending' THEN 1 ELSE 0 END
UNION
SELECT id FROM unresolved_refs
WHERE (?2 IN ('typescript','tsx','javascript','jsx') OR ?3=1)
    AND language IN ('typescript','tsx','javascript','jsx')
    AND reference_kind IN ('calls','function_ref','instantiates','extends','implements')
    AND (file_path,reference_name) IN (SELECT json_extract(value,'$[0]'),json_extract(value,'$[1]') FROM json_each(?5))
    AND CASE WHEN status='pending' THEN 1 ELSE 0 END
UNION
SELECT id FROM unresolved_refs
WHERE status='pending' AND (
    ((?2 IN ('typescript','tsx','javascript','jsx') OR ?3=1) AND language IN ('typescript','tsx','javascript','jsx')
        AND reference_kind IN ('angular_injects','angular_route_component','angular_route_layout'))
    OR ((?2 IN ('typescript','tsx','javascript','jsx') OR ?3=1)
        AND reference_kind IN ('angular_provider_token','angular_provider_implementation'))
    OR ((?2 IN ('typescript','tsx','javascript','jsx','vue') OR ?3=1) AND language='vue'
        AND reference_kind='calls' AND file_path IN (SELECT file_path FROM nodes WHERE kind='import'))
    OR (?2='rust' AND language='rust' AND reference_name LIKE 'super::%' AND reference_kind IN ('calls','function_ref'))
    OR (?2='html' AND reference_kind='framework_template' AND reference_name=?1)
)
ORDER BY id LIMIT 20001";

pub(super) const LEGACY_PENDING_SQL: &str = "SELECT id FROM unresolved_refs WHERE status='pending' AND (
reference_name IN(SELECT name FROM nodes WHERE file_path=?1 AND (?2 NOT IN ('typescript','tsx','javascript','jsx') OR ((visibility='public' OR ifnull(is_exported,0)<>0) AND kind IN ('function','method','class','struct','interface','trait','record','enum') AND name NOT IN(SELECT value FROM json_each(?4)))))
OR reference_name IN(SELECT qualified_name FROM nodes WHERE file_path=?1 AND (?2 NOT IN ('typescript','tsx','javascript','jsx') OR ((visibility='public' OR ifnull(is_exported,0)<>0) AND kind IN ('function','method','class','struct','interface','trait','record','enum') AND qualified_name NOT IN(SELECT value FROM json_each(?4)))))
OR (?2 IN ('typescript','tsx','javascript','jsx') AND language NOT IN ('typescript','tsx','javascript','jsx') AND (reference_name IN(SELECT name FROM nodes WHERE file_path=?1 AND (visibility='public' OR ifnull(is_exported,0)<>0) AND kind IN ('function','method','class','struct','interface','trait','record','enum')) OR reference_name IN(SELECT qualified_name FROM nodes WHERE file_path=?1 AND (visibility='public' OR ifnull(is_exported,0)<>0) AND kind IN ('function','method','class','struct','interface','trait','record','enum'))))
OR (file_path=?1 AND (reference_name IN(SELECT name FROM nodes WHERE file_path=?1) OR reference_name IN(SELECT qualified_name FROM nodes WHERE file_path=?1)))
OR ((?2 IN ('typescript','tsx','javascript','jsx') OR ?3=1) AND language IN ('typescript','tsx','javascript','jsx') AND ((?5 IS NULL AND file_path IN(SELECT file_path FROM nodes WHERE kind='import')) OR (reference_kind IN ('calls','function_ref','instantiates','extends','implements') AND (file_path,reference_name) IN(SELECT json_extract(value,'$[0]'),json_extract(value,'$[1]') FROM json_each(?5)))))
OR ((?2 IN ('typescript','tsx','javascript','jsx') OR ?3=1) AND language IN ('typescript','tsx','javascript','jsx') AND reference_kind='angular_injects')
OR ((?2 IN ('typescript','tsx','javascript','jsx') OR ?3=1) AND language IN ('typescript','tsx','javascript','jsx') AND reference_kind IN ('angular_route_component','angular_route_layout'))
OR ((?2 IN ('typescript','tsx','javascript','jsx','vue') OR ?3=1) AND language='vue' AND reference_kind='calls' AND file_path IN(SELECT file_path FROM nodes WHERE kind='import'))
OR (?2='rust' AND language='rust' AND reference_name LIKE 'super::%' AND reference_kind IN ('calls','function_ref'))
OR (?2='html' AND reference_kind='framework_template' AND reference_name=?1)) LIMIT 20001";

pub(super) fn pending(
    db: &Transaction<'_>,
    path: &str,
    language: &str,
    config_changed: bool,
    attempted_names: &str,
    import_bindings: Option<&str>,
) -> Result<Vec<i64>, Failure> {
    // Diagnostic rollback switch for controlled A/B runs; both paths preserve
    // selection and the overflow sentinel. Default to indexed selective seeks.
    let query = if uses_legacy_pending() {
        LEGACY_PENDING_SQL
    } else {
        PENDING_SQL
    };
    let mut statement = sql(db.prepare_cached(query))?;
    let rows = sql(statement.query_map(
        rusqlite::params![
            path,
            language,
            config_changed,
            attempted_names,
            import_bindings
        ],
        |row| row.get(0),
    ))?;
    sql(rows.collect())
}

pub(super) fn uses_legacy_pending() -> bool {
    std::env::var("HARNESS_INDEX_PENDING_QUERY").as_deref() == Ok("legacy")
}

pub(super) type Declaration = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<bool>,
    String,
    i64,
);

pub(super) fn visible_names(declarations: &[Declaration]) -> Vec<String> {
    declarations
        .iter()
        .filter(|row| {
            (row.4.as_deref() == Some("public") || row.5 == Some(true))
                && matches!(
                    row.1.as_str(),
                    "function"
                        | "method"
                        | "class"
                        | "struct"
                        | "interface"
                        | "trait"
                        | "record"
                        | "enum"
                )
        })
        .flat_map(|row| [row.2.clone(), row.3.clone()])
        .collect()
}

/// Exact generic-lookup inputs. Large snapshots fall back to ordinary retries.
pub(super) fn declarations(
    db: &Transaction<'_>,
    path: &str,
) -> Result<Option<Vec<Declaration>>, Failure> {
    let mut statement = sql(db.prepare_cached("SELECT id,kind,name,qualified_name,visibility,is_exported,language,start_line FROM nodes WHERE file_path=?1 ORDER BY id LIMIT 20001"))?;
    let rows = sql(statement.query_map([path], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
        ))
    }))?;
    let mut result: Vec<Declaration> = Vec::new();
    let mut reserved = 0usize;
    for row in rows {
        let row: Declaration = sql(row)?;
        reserved += row.0.len()
            + row.1.len()
            + row.2.len()
            + row.3.len()
            + row.4.as_ref().map_or(0, String::len)
            + row.6.len()
            + 256;
        if result.len() >= 20_000 || reserved > 2 * 1024 * 1024 {
            return Ok(None);
        }
        result.push(row);
    }
    Ok(Some(result))
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pending_optimization_tests.rs"]
mod optimization_tests;
