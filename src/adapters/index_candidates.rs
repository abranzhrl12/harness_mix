//! Raw candidate lookup cache for one publication's immutable node snapshot.
//! Construct AFTER node writes and discard BEFORE publishing the next file.
//! Never cache caller-specific visibility, lexical binding or export decisions.
use super::sql;
use harness_domain::{Failure, execution::OperationContext};
use rusqlite::{Connection, params};
use std::collections::BTreeMap;

type Candidate = (String, String, String, String, Option<String>, bool, String);
type Key = (String, String, Option<String>);
const MAX_ENTRIES: usize = 128;
const MAX_BYTES: usize = 2 * 1024 * 1024;
// Seek the two symbol indexes first; avoid the OR/filter/order file-index scan.
// Each ordered branch is capped before UNION: at most 130 rows are combined.
const QUERY: &str = "WITH by_name AS (SELECT id,kind,file_path,qualified_name,visibility,is_exported,language,start_line FROM nodes WHERE name=?1 COLLATE BINARY AND ((?3 IS NULL AND language=?2) OR (?3 IS NOT NULL AND file_path=?3 COLLATE BINARY AND language IN ('typescript','tsx','javascript','jsx'))) ORDER BY file_path,start_line,id LIMIT 65), by_qualified AS (SELECT id,kind,file_path,qualified_name,visibility,is_exported,language,start_line FROM nodes WHERE qualified_name=?1 COLLATE BINARY AND ((?3 IS NULL AND language=?2) OR (?3 IS NOT NULL AND file_path=?3 COLLATE BINARY AND language IN ('typescript','tsx','javascript','jsx'))) ORDER BY file_path,start_line,id LIMIT 65) SELECT id,kind,file_path,qualified_name,visibility,is_exported,language FROM (SELECT * FROM by_name UNION SELECT * FROM by_qualified) ORDER BY file_path,start_line,id LIMIT 65";

#[derive(Default)]
pub(super) struct Snapshot {
    entries: BTreeMap<Key, Vec<Candidate>>,
    bytes: usize,
    #[cfg(test)]
    queries: usize,
}

impl Snapshot {
    pub(super) fn lookup(
        &mut self,
        db: &Connection,
        name: &str,
        language: &str,
        imported_path: Option<&str>,
        context: &OperationContext,
    ) -> Result<Vec<Candidate>, Failure> {
        context.check()?;
        let key = (
            name.to_owned(),
            language.to_owned(),
            imported_path.map(str::to_owned),
        );
        if let Some(rows) = self.entries.get(&key) {
            return Ok(rows.clone());
        }
        let rows = query(db, name, language, imported_path)?;
        #[cfg(test)]
        {
            self.queries += 1;
        }
        // Account retained vector/string allocations, plus conservative map
        // overhead. Oversize or saturated caches fall back to the SAME query.
        let bytes = 256
            + key.0.capacity()
            + key.1.capacity()
            + key.2.as_ref().map_or(0, String::capacity)
            + rows.capacity() * std::mem::size_of::<Candidate>()
            + rows
                .iter()
                .map(|n| {
                    n.0.capacity()
                        + n.1.capacity()
                        + n.2.capacity()
                        + n.3.capacity()
                        + n.4.as_ref().map_or(0, String::capacity)
                        + n.6.capacity()
                })
                .sum::<usize>();
        if self.entries.len() < MAX_ENTRIES && bytes <= MAX_BYTES.saturating_sub(self.bytes) {
            self.bytes += bytes;
            self.entries.insert(key, rows.clone());
        }
        Ok(rows)
    }
}

fn query(
    db: &Connection,
    name: &str,
    language: &str,
    imported_path: Option<&str>,
) -> Result<Vec<Candidate>, Failure> {
    let mut statement = sql(db.prepare_cached(QUERY))?;
    let rows = sql(
        statement.query_map(params![name, language, imported_path], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get::<_, Option<bool>>(5)?.unwrap_or(false),
                r.get(6)?,
            ))
        }),
    )?;
    sql(rows.collect::<rusqlite::Result<Vec<_>>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_branches_match_global_limit_with_overlap_and_disjoint_matches() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        for i in 0..300 {
            let name = if i % 3 != 1 {
                "key".to_owned()
            } else {
                format!("other{i}")
            };
            let qualified = if i % 3 != 0 {
                "key".to_owned()
            } else {
                format!("Owner{i}::key")
            };
            tx.execute(
                "INSERT INTO nodes VALUES(?1,'function',?2,?3,'a.rs','public',1,'rust',?4)",
                params![format!("merge{i:03}"), name, qualified, 299 - i],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        let actual = query(&db, "key", "rust", None).unwrap();
        assert_eq!(actual, legacy_query(&db, "key", "rust", None));
        assert_eq!(actual.len(), 65);
        assert_eq!(
            actual
                .iter()
                .map(|row| &row.0)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            65
        );
    }

    #[test]
    fn lookup_does_not_depend_on_an_unshadowed_sqlite_rowid_alias() {
        let db = fixture();
        db.execute_batch("ALTER TABLE nodes ADD COLUMN rowid INTEGER")
            .unwrap();
        assert_eq!(
            query(&db, "target", "rust", None).unwrap(),
            legacy_query(&db, "target", "rust", None)
        );
        assert_eq!(query(&db, "target", "rust", None).unwrap().len(), 1);
    }

    fn homonyms(count: usize) -> Connection {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        for i in 0..count {
            let (path, language) = if i % 3 == 0 {
                ("a.ts", "typescript")
            } else {
                ("b.rs", "rust")
            };
            let qualified = if i % 2 == 0 {
                "shared".to_owned()
            } else {
                format!("Owner{i}::shared")
            };
            tx.execute(
                "INSERT INTO nodes VALUES(?1,'function','shared',?2,?3,'public',1,?4,?5)",
                params![
                    format!("h{i:05}"),
                    qualified,
                    path,
                    language,
                    ((i * 17) % 23) as i64
                ],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        db
    }

    #[test]
    fn thousands_of_homonyms_preserve_exact_rows_order_and_ceiling() {
        let db = homonyms(8192);
        for (language, path) in [
            ("rust", None),
            ("typescript", None),
            ("javascript", Some("a.ts")),
            ("rust", Some("b.rs")),
        ] {
            let start = std::time::Instant::now();
            let old = legacy_query(&db, "shared", language, path);
            let old_time = start.elapsed();
            let start = std::time::Instant::now();
            let current = query(&db, "shared", language, path).unwrap();
            let current_time = start.elapsed();
            assert_eq!(current, old);
            assert_eq!(current.len(), if path == Some("b.rs") { 0 } else { 65 });
            println!(
                "{}",
                serde_json::json!({"component":"many-homonyms","nodes":8192,"language":language,"importedPath":path,"legacyMs":old_time.as_secs_f64()*1000.0,"indexedMs":current_time.as_secs_f64()*1000.0,"returnedRows":current.len(),"exactRowsAndOrderMatch":true,"debugBuild":cfg!(debug_assertions)})
            );
        }
    }

    #[test]
    fn cancellation_and_deadlines_apply_even_to_cached_candidates() {
        let db = fixture();
        let context = OperationContext::default();
        let mut cache = Snapshot::default();
        cache.lookup(&db, "target", "rust", None, &context).unwrap();
        context.cancel();
        assert_eq!(
            cache
                .lookup(&db, "target", "rust", None, &context)
                .unwrap_err()
                .0,
            "CANCELLED"
        );
        let expired = OperationContext::with_timeout(std::time::Duration::ZERO);
        assert_eq!(
            cache
                .lookup(&db, "missing", "rust", None, &expired)
                .unwrap_err()
                .0,
            "TIMEOUT"
        );
        assert_eq!(cache.queries, 1);
    }

    #[test]
    fn interrupted_sql_never_populates_the_candidate_cache() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let db = homonyms(4096);
        let context = OperationContext::default();
        let cancel = context.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let callback_calls = calls.clone();
        db.progress_handler(
            1,
            Some(move || {
                if callback_calls.fetch_add(1, Ordering::Relaxed) >= 64 {
                    cancel.cancel();
                    true
                } else {
                    false
                }
            }),
        )
        .unwrap();
        let mut cache = Snapshot::default();
        assert!(cache.lookup(&db, "shared", "rust", None, &context).is_err());
        db.progress_handler(0, None::<fn() -> bool>).unwrap();
        assert_eq!(context.check().unwrap_err().0, "CANCELLED");
        assert!(cache.entries.is_empty());
        assert_eq!(cache.queries, 0);
        assert!(db.is_autocommit());
        assert!(calls.load(Ordering::Relaxed) >= 65);
    }

    // Retain the pre-optimization SQL as an independent test oracle.
    const LEGACY_QUERY: &str = "SELECT id,kind,file_path,qualified_name,visibility,is_exported,language FROM nodes WHERE (name=?1 COLLATE BINARY OR qualified_name=?1 COLLATE BINARY) AND ((?3 IS NULL AND language=?2) OR (?3 IS NOT NULL AND file_path=?3 COLLATE BINARY AND language IN ('typescript','tsx','javascript','jsx'))) ORDER BY file_path,start_line,id LIMIT 65";

    fn legacy_query(
        db: &Connection,
        name: &str,
        language: &str,
        path: Option<&str>,
    ) -> Vec<Candidate> {
        db.prepare_cached(LEGACY_QUERY)
            .unwrap()
            .query_map(params![name, language, path], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get::<_, Option<bool>>(5)?.unwrap_or(false),
                    r.get(6)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn indexed_query_matches_legacy_across_scopes() {
        let db = fixture();
        db.execute("INSERT INTO nodes VALUES('alias','method','alias','target','a.rs','private',0,'rust',0)", []).unwrap();
        for i in 0..75 {
            db.execute(
                "INSERT INTO nodes VALUES(?1,'function','many','many','a.rs','public',1,'rust',?2)",
                params![format!("many{i}"), 75 - i],
            )
            .unwrap();
        }
        for name in ["target", "Target", "missing", "many", "alias"] {
            for language in ["rust", "typescript", "javascript"] {
                for path in [None, Some("a.ts"), Some("b.js"), Some("absent.ts")] {
                    assert_eq!(
                        legacy_query(&db, name, language, path),
                        query(&db, name, language, path).unwrap()
                    );
                }
            }
        }
        assert_eq!(legacy_query(&db, "many", "rust", None).len(), 65);
    }

    fn fixture() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE nodes(id TEXT,kind TEXT,name TEXT,qualified_name TEXT,file_path TEXT,visibility TEXT,is_exported INTEGER,language TEXT,start_line INTEGER); CREATE INDEX names ON nodes(name); CREATE INDEX qualified ON nodes(qualified_name);").unwrap();
        for (id, path, language) in [
            ("r", "a.rs", "rust"),
            ("t", "a.ts", "typescript"),
            ("j", "b.js", "javascript"),
        ] {
            db.execute(
                "INSERT INTO nodes VALUES(?1,'function','target','target',?2,'public',1,?3,1)",
                params![id, path, language],
            )
            .unwrap();
        }
        db
    }

    #[test]
    fn cache_keys_preserve_language_import_path_and_case() {
        let db = fixture();
        let mut cache = Snapshot::default();
        let context = OperationContext::default();
        for name in ["target", "Target", "missing"] {
            for language in ["rust", "typescript", "javascript"] {
                for path in [None, Some("a.ts"), Some("b.js"), Some("absent.ts")] {
                    let expected = query(&db, name, language, path).unwrap();
                    assert_eq!(
                        cache.lookup(&db, name, language, path, &context).unwrap(),
                        expected
                    );
                    assert_eq!(
                        cache.lookup(&db, name, language, path, &context).unwrap(),
                        expected
                    );
                }
            }
        }
        assert_eq!(cache.queries, 36);
    }

    #[test]
    fn raw_rows_are_not_changed_by_caller_filtering_and_reset_sees_new_nodes() {
        let db = fixture();
        let context = OperationContext::default();
        let mut cache = Snapshot::default();
        let mut rows = cache.lookup(&db, "target", "rust", None, &context).unwrap();
        rows.clear();
        assert_eq!(
            cache
                .lookup(&db, "target", "rust", None, &context)
                .unwrap()
                .len(),
            1
        );
        assert!(
            cache
                .lookup(&db, "new", "rust", None, &context)
                .unwrap()
                .is_empty()
        );
        // Nodes are only mutated BETWEEN snapshots in production.
        db.execute(
            "INSERT INTO nodes VALUES('new','function','new','new','a.rs','public',1,'rust',2)",
            [],
        )
        .unwrap();
        let mut next = Snapshot::default();
        assert_eq!(
            next.lookup(&db, "new", "rust", None, &context)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn ambiguity_sentinel_and_row_order_match_uncached_lookup() {
        let db = fixture();
        for i in 0..70 {
            db.execute(
                "INSERT INTO nodes VALUES(?1,'function','many','many','a.rs','public',1,'rust',?2)",
                params![i.to_string(), i],
            )
            .unwrap();
        }
        let context = OperationContext::default();
        let mut cache = Snapshot::default();
        let expected = query(&db, "many", "rust", None).unwrap();
        assert_eq!(expected.len(), 65);
        assert_eq!(
            cache.lookup(&db, "many", "rust", None, &context).unwrap(),
            expected
        );
        assert_eq!(
            cache.lookup(&db, "many", "rust", None, &context).unwrap(),
            expected
        );
        assert_eq!(cache.queries, 1);
    }

    #[test]
    fn bounded_cache_falls_back_without_losing_candidates() {
        let db = fixture();
        let context = OperationContext::default();
        let mut cache = Snapshot::default();
        for i in 0..MAX_ENTRIES + 10 {
            assert!(
                cache
                    .lookup(&db, &format!("missing{i}"), "rust", None, &context)
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache.bytes <= MAX_BYTES);
        assert_eq!(
            cache.lookup(&db, "target", "rust", None, &context).unwrap(),
            query(&db, "target", "rust", None).unwrap()
        );
        let huge = "x".repeat(MAX_BYTES + 1);
        db.execute(
            "INSERT INTO nodes VALUES(?1,'function','huge','huge','a.rs','public',1,'rust',1)",
            [&huge],
        )
        .unwrap();
        let mut small = Snapshot::default();
        assert_eq!(
            small.lookup(&db, "huge", "rust", None, &context).unwrap()[0].0,
            huge
        );
        assert!(small.entries.is_empty());
    }

    #[test]
    fn repeated_lookup_diagnostic() {
        let db = fixture();
        let context = OperationContext::default();
        let names = ["target", "missing", "Target", "unknown"];
        for round in 0..3 {
            let started = std::time::Instant::now();
            for i in 0..4096 {
                std::hint::black_box(query(&db, names[i % 4], "rust", None).unwrap());
            }
            let direct = started.elapsed();
            let mut cache = Snapshot::default();
            let started = std::time::Instant::now();
            for i in 0..4096 {
                std::hint::black_box(
                    cache
                        .lookup(&db, names[i % 4], "rust", None, &context)
                        .unwrap(),
                );
            }
            let cached = started.elapsed();
            assert_eq!(cache.queries, 4);
            println!(
                "{}",
                serde_json::json!({"component":"candidate-lookup-only","round":round+1,"lookups":4096,"uncachedSqlQueries":4096,"cachedSqlQueries":cache.queries,"uncachedMs":direct.as_secs_f64()*1000.0,"cachedMs":cached.as_secs_f64()*1000.0,"debugBuild":cfg!(debug_assertions),"wholeIndexSpeedupClaimed":false})
            );
        }
    }

    #[test]
    fn unique_lookup_diagnostic() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../../vendor/codegraph-native/schema.sql"))
            .unwrap();
        let present = (0..4096).map(|i| format!("unique{i}")).collect::<Vec<_>>();
        let absent = (0..512).map(|i| format!("absent{i}")).collect::<Vec<_>>();
        {
            let tx = db.transaction().unwrap();
            for name in &present {
                tx.execute("INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES(?1,'function',?1,?1,'a.rs','rust',1,1,0,1,0)", [name]).unwrap();
            }
            tx.commit().unwrap();
        }
        let plan = db
            .prepare(&format!("EXPLAIN QUERY PLAN {QUERY}"))
            .unwrap()
            .query_map(params!["unique0", "rust", None::<&str>], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(plan.iter().any(|step| step.contains("idx_nodes_name")));
        assert!(
            plan.iter()
                .any(|step| step.contains("idx_nodes_qualified_name"))
        );
        assert!(!plan.iter().any(|step| step.contains("SCAN nodes")));
        println!(
            "{}",
            serde_json::json!({"component":"candidate-query-plan-applied","realSchema":true,"indexedNodes":4096,"plan":plan})
        );
        for (workload, names) in [
            ("present-unique", &present[..512]),
            ("absent-unique", &absent[..]),
        ] {
            for round in 0..3 {
                let old = || {
                    let start = std::time::Instant::now();
                    for name in names {
                        std::hint::black_box(legacy_query(&db, name, "rust", None));
                    }
                    start.elapsed()
                };
                let current = || {
                    let start = std::time::Instant::now();
                    for name in names {
                        std::hint::black_box(query(&db, name, "rust", None).unwrap());
                    }
                    start.elapsed()
                };
                let (old, current) = if round % 2 == 0 {
                    let current = current();
                    (old(), current)
                } else {
                    let old = old();
                    (old, current())
                };
                for name in [&names[0], &names[127], &names[511]] {
                    assert_eq!(
                        query(&db, name, "rust", None).unwrap(),
                        legacy_query(&db, name, "rust", None)
                    );
                }
                println!(
                    "{}",
                    serde_json::json!({"component":"candidate-lookup-indexed-applied","workload":workload,"round":round+1,"lookups":names.len(),"legacyMs":old.as_secs_f64()*1000.0,"indexedMs":current.as_secs_f64()*1000.0,"debugBuild":cfg!(debug_assertions),"productionQueryChanged":true,"wholeIndexSpeedupClaimed":false})
                );
            }
        }
    }
}
