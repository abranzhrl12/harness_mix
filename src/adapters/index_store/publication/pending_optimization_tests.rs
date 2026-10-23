use super::*;
use rusqlite::{Connection, StatementStatus, params};

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE nodes(
            id TEXT PRIMARY KEY, file_path TEXT, language TEXT, kind TEXT,
            name TEXT, qualified_name TEXT, visibility TEXT, is_exported INTEGER
         );
         CREATE TABLE unresolved_refs(
            id INTEGER PRIMARY KEY, from_node_id TEXT, reference_name TEXT,
            reference_kind TEXT, line INTEGER, col INTEGER, candidates TEXT,
            file_path TEXT, language TEXT, status TEXT, name_tail TEXT
         );
         CREATE INDEX idx_unresolved_name ON unresolved_refs(reference_name);
         CREATE INDEX idx_unresolved_file_path ON unresolved_refs(file_path);
         CREATE INDEX idx_unresolved_status ON unresolved_refs(status);
         CREATE INDEX idx_nodes_file_path ON nodes(file_path);
         CREATE INDEX idx_nodes_kind ON nodes(kind,file_path);
         INSERT INTO nodes VALUES
            ('pub','changed.js','javascript','function','public_name','ns::public_name','public',1),
            ('priv','changed.js','javascript','function','private_name','ns::private_name',NULL,0),
            ('import','importer.js','javascript','import','local_alias','local_alias',NULL,0),
            ('vue-import','vue-importer.vue','vue','import','vue_alias','vue_alias',NULL,0),
            ('owner','changed.js','javascript','function','owner','owner',NULL,0),
            ('null-name','changed.js','javascript','function',NULL,NULL,NULL,0);
         INSERT INTO unresolved_refs VALUES
            (1,'owner','public_name','calls',1,1,NULL,'caller.rs','rust','pending',''),
            (2,'owner','ns::public_name','function_ref',2,1,NULL,'caller.rs','rust','pending',''),
            (3,'owner','private_name','calls',3,1,NULL,'caller.rs','rust','pending',''),
            (4,'owner','public_name','calls',4,1,NULL,'changed.js','javascript','pending',''),
            (5,'owner','public_name','calls',5,1,NULL,'caller.js','javascript','failed',''),
            (6,'owner','local_alias','calls',6,1,NULL,'importer.js','javascript','pending',''),
            (7,'owner','public_name','angular_injects',7,1,NULL,'caller.ts','typescript','pending',''),
            (8,'owner','RouteThing','angular_route_component',8,1,NULL,'caller.ts','typescript','pending',''),
            (9,'owner','RouteThing','angular_route_layout',9,1,NULL,'caller.ts','typescript','pending',''),
            (10,'owner','ProviderThing','angular_provider_token',10,1,NULL,'caller.ts','typescript','pending',''),
            (11,'owner','ProviderThing','angular_provider_implementation',11,1,NULL,'caller.ts','typescript','pending',''),
            (12,'owner','composable_call','calls',12,1,NULL,'vue-importer.vue','vue','pending',''),
            (13,'owner','super::method','calls',13,1,NULL,'caller.rs','rust','pending',''),
            (14,'owner','super::method','function_ref',14,1,NULL,'caller.rs','rust','pending',''),
            (15,'owner','template.html','framework_template',15,1,NULL,'caller.html','html','pending',''),
            (16,'owner','not_exported','calls',16,1,NULL,'caller.js','javascript','pending',''),
            (17,'owner','public_name','calls',17,1,NULL,'caller.ts','typescript','pending',''),
            (18,'owner','ProviderRust','angular_provider_token',18,1,NULL,'rust-provider.rs','rust','pending',''),
            (19,'owner','ProviderVue','angular_provider_implementation',19,1,NULL,'vue-provider.vue','vue','pending',''),
            (20,'owner','ProviderUnknown','angular_provider_token',20,1,NULL,'unknown-provider',NULL,'pending',''),
            (21,'owner','ProviderFailed','angular_provider_token',21,1,NULL,'failed-provider.ts','typescript','failed',''),
            (22,'owner',NULL,'calls',22,1,NULL,'caller.rs','rust','pending','');",
    )
    .unwrap();
    db
}

fn query_ids(
    db: &Connection,
    query: &str,
    path: &str,
    language: &str,
    config_changed: bool,
    attempted_names: &str,
    import_bindings: Option<&str>,
) -> Vec<i64> {
    let mut statement = db.prepare(query).unwrap();
    let rows = statement
        .query_map(
            params![
                path,
                language,
                config_changed,
                attempted_names,
                import_bindings
            ],
            |row| row.get(0),
        )
        .unwrap();
    rows.map(Result::unwrap).collect()
}

fn effective_legacy_query(language: &str, config_changed: bool) -> String {
    let providers_enabled =
        config_changed || matches!(language, "typescript" | "tsx" | "javascript" | "jsx");
    if !providers_enabled {
        return LEGACY_PENDING_SQL.to_owned();
    }
    format!(
        "SELECT id FROM ({LEGACY_PENDING_SQL}) UNION SELECT id FROM unresolved_refs \
         WHERE status='pending' AND reference_kind IN \
         ('angular_provider_token','angular_provider_implementation') LIMIT 20001"
    )
}

/// Test-only candidate: split the final conservative OR into independently
/// gated UNION branches. Provider refs intentionally remain ungated by language.
fn split_conservative_query() -> String {
    let tail_start = PENDING_SQL
        .rfind("SELECT id FROM unresolved_refs")
        .expect("final conservative branch exists");
    let prefix = &PENDING_SQL[..tail_start];
    format!(
        "{prefix}\nSELECT id FROM unresolved_refs WHERE status='pending'\n\
         AND (?2 IN ('typescript','tsx','javascript','jsx') OR ?3=1)\n\
         AND language IN ('typescript','tsx','javascript','jsx')\n\
         AND reference_kind IN ('angular_injects','angular_route_component','angular_route_layout')\n\
         UNION\nSELECT id FROM unresolved_refs WHERE status='pending'\n\
         AND (?2 IN ('typescript','tsx','javascript','jsx') OR ?3=1)\n\
         AND reference_kind IN ('angular_provider_token','angular_provider_implementation')\n\
         UNION\nSELECT id FROM unresolved_refs WHERE status='pending'\n\
         AND (?2 IN ('typescript','tsx','javascript','jsx','vue') OR ?3=1)\n\
         AND language='vue' AND reference_kind='calls'\n\
         AND file_path IN (SELECT file_path FROM nodes WHERE kind='import')\n\
         UNION\nSELECT id FROM unresolved_refs WHERE status='pending'\n\
         AND ?2='rust' AND language='rust'\n\
         AND reference_name LIKE 'super::%' AND reference_kind IN ('calls','function_ref')\n\
         UNION\nSELECT id FROM unresolved_refs WHERE status='pending'\n\
         AND ?2='html' AND reference_kind='framework_template' AND reference_name=?1\n\
         ORDER BY id LIMIT 20001"
    )
}

fn assert_equivalent(
    db: &Connection,
    path: &str,
    language: &str,
    config_changed: bool,
    attempted_names: &str,
    import_bindings: Option<&str>,
) {
    let legacy_query = effective_legacy_query(language, config_changed);
    let mut legacy = query_ids(
        db,
        &legacy_query,
        path,
        language,
        config_changed,
        attempted_names,
        import_bindings,
    );
    let current = query_ids(
        db,
        PENDING_SQL,
        path,
        language,
        config_changed,
        attempted_names,
        import_bindings,
    );
    legacy.sort_unstable();
    let split = query_ids(
        db,
        &split_conservative_query(),
        path,
        language,
        config_changed,
        attempted_names,
        import_bindings,
    );
    let routed_sql = pending_sql_for(language, config_changed, false, true);
    let routed = query_ids(
        db,
        routed_sql,
        path,
        language,
        config_changed,
        attempted_names,
        import_bindings,
    );
    assert_eq!(
        current, legacy,
        "{language}, config={config_changed}, names={attempted_names}, bindings={import_bindings:?}"
    );
    assert_eq!(
        split, current,
        "split conservative branches differ: {language}, config={config_changed}, names={attempted_names}, bindings={import_bindings:?}"
    );
    assert_eq!(routed, current, "routed query differs: {language}, config={config_changed}");
    assert_eq!(
        routed_sql,
        if language == "rust" && !config_changed { RUST_PENDING_SQL } else { PENDING_SQL },
        "unexpected pending SQL route: {language}, config={config_changed}"
    );
}

#[test]
fn pending_query_route_is_rust_only_and_legacy_has_precedence() {
    assert_eq!(
        pending_sql_for("rust", false, false, true),
        RUST_PENDING_SQL
    );
    assert_eq!(
        pending_sql_for("rust", false, true, true),
        LEGACY_PENDING_SQL,
        "legacy selector must override the Rust optimization"
    );
    assert_eq!(pending_sql_for("rust", false, false, false), PENDING_SQL);
    assert_eq!(pending_sql_for("rust", true, false, true), PENDING_SQL);
    for language in ["javascript", "typescript", "vue", "html", "csharp"] {
        for config_changed in [false, true] {
            assert_eq!(
                pending_sql_for(language, config_changed, false, true),
                PENDING_SQL,
                "non-Rust profile routed unexpectedly: {language}, config={config_changed}"
            );
        }
    }
}

#[test]
fn pending_query_matches_legacy_across_language_and_catalog_matrix() {
    let db = fixture();
    let catalog = [
        None,
        Some("null"),
        Some("[]"),
        Some(r#"[["importer.js","local_alias"],["importer.js","public_name"]]"#),
        Some(
            r#"[["importer.js","local_alias"],["importer.js","local_alias"],["importer.js","public_name"]]"#,
        ),
    ];
    let attempted = [
        "[]",
        "null",
        "[null]",
        r#"["public_name","ns::public_name"]"#,
        r#"["public_name","public_name","ns::public_name"]"#,
    ];
    for language in ["javascript", "typescript", "rust", "vue", "html", "csharp"] {
        for config_changed in [false, true] {
            for names in attempted {
                for bindings in catalog {
                    assert_equivalent(&db, "changed.js", language, config_changed, names, bindings);
                }
            }
        }
    }
    // Also exercise each special source path directly, including local names
    // retained for the resolver's homonym/candidate ceiling.
    for (path, language) in [
        ("changed.js", "javascript"),
        ("importer.js", "typescript"),
        ("vue-importer.vue", "vue"),
        ("caller.rs", "rust"),
        ("caller.html", "html"),
    ] {
        assert_equivalent(&db, path, language, false, "[]", None);
        assert_equivalent(
            &db,
            path,
            language,
            true,
            r#"["public_name","local_alias"]"#,
            Some(r#"[["importer.js","local_alias"],["vue-importer.vue","vue_alias"]]"#),
        );
    }
    // The HTML fallback compares the reference name to the changed path,
    // rather than to a declaration name in the changed file.
    assert_equivalent(&db, "template.html", "html", false, "[]", None);
    assert_equivalent(&db, "template.html", "html", true, "[]", Some("null"));
}

#[test]
fn pending_query_preserves_limit_signal_above_max_rows() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE nodes(file_path TEXT,kind TEXT,name TEXT,qualified_name TEXT,visibility TEXT,is_exported INTEGER);
         CREATE TABLE unresolved_refs(id INTEGER PRIMARY KEY,reference_name TEXT,file_path TEXT,language TEXT,reference_kind TEXT,status TEXT);
         CREATE INDEX idx_unresolved_status ON unresolved_refs(status);
         CREATE INDEX idx_unresolved_name ON unresolved_refs(reference_name);
         CREATE INDEX idx_unresolved_file_path ON unresolved_refs(file_path);
         CREATE INDEX idx_nodes_file_path ON nodes(file_path);
         INSERT INTO nodes VALUES('changed.js','function','same','same',NULL,0);",
    )
    .unwrap();
    for id in 1..=20_002 {
        db.execute(
            "INSERT INTO unresolved_refs VALUES(?1,'super::same','changed.js','rust','calls','pending')",
            [id],
        )
        .unwrap();
    }
    let old_query = effective_legacy_query("rust", false);
    let mut old = query_ids(
        &db,
        &old_query,
        "changed.js",
        "rust",
        false,
        "[]",
        None,
    );
    let new = query_ids(
        &db,
        PENDING_SQL,
        "changed.js",
        "rust",
        false,
        "[]",
        None,
    );
    old.sort_unstable();
    let split = query_ids(
        &db,
        &split_conservative_query(),
        "changed.js",
        "rust",
        false,
        "[]",
        None,
    );
    let specialized = query_ids(
        &db,
        RUST_PENDING_SQL,
        "changed.js",
        "rust",
        false,
        "[]",
        None,
    );
    assert_eq!(old.len(), 20_001);
    assert_eq!(new.len(), 20_001);
    assert_eq!(old, new);
    assert_eq!(new, split);
    assert_eq!(specialized.len(), 20_001);
    assert_eq!(new, specialized);
}

#[test]
fn pending_query_reports_vm_step_comparison_on_large_unrelated_fixture() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE nodes(file_path TEXT,kind TEXT,name TEXT,qualified_name TEXT,visibility TEXT,is_exported INTEGER);
         CREATE TABLE unresolved_refs(id INTEGER PRIMARY KEY,reference_name TEXT,file_path TEXT,language TEXT,reference_kind TEXT,status TEXT);
         CREATE INDEX idx_unresolved_status ON unresolved_refs(status);
         CREATE INDEX idx_unresolved_name ON unresolved_refs(reference_name);
         CREATE INDEX idx_unresolved_file_path ON unresolved_refs(file_path);
         CREATE INDEX idx_nodes_file_path ON nodes(file_path);
         INSERT INTO nodes VALUES('changed.js','function','hit','hit','public',1);",
    )
    .unwrap();
    for id in 1..=20_000 {
        db.execute(
            "INSERT INTO unresolved_refs VALUES(?1,'unrelated','other.js','javascript','calls','pending')",
            [id],
        )
        .unwrap();
    }
    db.execute_batch(
        "INSERT INTO unresolved_refs VALUES(20001,'hit','other.js','javascript','calls','pending');
         INSERT INTO unresolved_refs VALUES(20002,'hit','changed.js','javascript','calls','pending');",
    )
    .unwrap();
    let mut measurements = Vec::new();
    let split = split_conservative_query();
    for query in [LEGACY_PENDING_SQL, PENDING_SQL, split.as_str()] {
        let mut statement = db.prepare(query).unwrap();
        let ids: Vec<i64> = statement
            .query_map(
                params!["changed.js", "javascript", false, "[]", None::<&str>],
                |row| row.get(0),
            )
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let vm_steps = statement.get_status(StatementStatus::VmStep);
        measurements.push((ids, vm_steps));
    }
    measurements[0].0.sort_unstable();
    assert_eq!(measurements[0].0, measurements[1].0);
    assert_eq!(measurements[1].0, measurements[2].0);
    eprintln!(
        "pending VM steps: legacy={}, current={}, split={}",
        measurements[0].1, measurements[1].1, measurements[2].1
    );
    assert!(
        measurements[1].1 * 2 < measurements[0].1,
        "expected at least 50% fewer VM steps: legacy={}, current={}",
        measurements[0].1,
        measurements[1].1
    );
}

#[test]
fn split_conservative_provider_heavy_vm_and_plan_benchmark() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE nodes(file_path TEXT,kind TEXT,name TEXT,qualified_name TEXT,visibility TEXT,is_exported INTEGER);
         CREATE TABLE unresolved_refs(id INTEGER PRIMARY KEY,reference_name TEXT,file_path TEXT,language TEXT,reference_kind TEXT,status TEXT);
         CREATE INDEX idx_unresolved_status ON unresolved_refs(status);
         CREATE INDEX idx_unresolved_name ON unresolved_refs(reference_name);
         CREATE INDEX idx_unresolved_file_path ON unresolved_refs(file_path);
         CREATE INDEX idx_nodes_file_path ON nodes(file_path);
         CREATE INDEX idx_nodes_kind ON nodes(kind,file_path);
         INSERT INTO nodes VALUES('changed.ts','class','Thing','Thing','public',1),('importer.ts','import','Thing','Thing',NULL,0);",
    )
    .unwrap();
    for id in 1..=20_000 {
        db.execute(
            "INSERT INTO unresolved_refs VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                id,
                "unrelated",
                if id % 2 == 0 { "unrelated.ts" } else { "unrelated.js" },
                if id % 3 == 0 { "rust" } else { "typescript" },
                if id % 4 == 0 { "angular_provider_token" } else { "calls" },
                if id % 5 == 0 { "failed" } else { "pending" }
            ],
        )
        .unwrap();
    }
    db.execute_batch(
        "INSERT INTO unresolved_refs VALUES(20001,'Thing','elsewhere.ts','typescript','angular_injects','pending');
         INSERT INTO unresolved_refs VALUES(20002,'provider','elsewhere.rs','rust','angular_provider_implementation','pending');
         INSERT INTO unresolved_refs VALUES(20003,'super::Thing','elsewhere.rs','rust','calls','pending');",
    )
    .unwrap();

    let split = split_conservative_query();
    let queries = [PENDING_SQL, split.as_str()];
    let mut expected: [Option<Vec<i64>>; 2] = [None, None];
    for (index, query) in queries.iter().enumerate() {
        let mut statement = db.prepare(query).unwrap();
        expected[index] = Some(
            statement
                .query_map(
                    params!["changed.ts", "typescript", false, "[]", None::<&str>],
                    |row| row.get(0),
                )
                .unwrap()
                .map(Result::unwrap)
                .collect(),
        );
    }
    assert_eq!(expected[0], expected[1]);

    let mut times: [Vec<std::time::Duration>; 2] = [Vec::new(), Vec::new()];
    let mut vm_steps = [0; 2];
    for repetition in 0..5 {
        let order = if repetition % 2 == 0 { [0, 1] } else { [1, 0] };
        for index in order {
            let mut statement = db.prepare(queries[index]).unwrap();
            let started = std::time::Instant::now();
            let ids: Vec<i64> = statement
                .query_map(
                    params!["changed.ts", "typescript", false, "[]", None::<&str>],
                    |row| row.get(0),
                )
                .unwrap()
                .map(Result::unwrap)
                .collect();
            times[index].push(started.elapsed());
            vm_steps[index] = statement.get_status(StatementStatus::VmStep);
            assert_eq!(Some(&ids), expected[index].as_ref());
            assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }
    let mut plans: [String; 2] = std::array::from_fn(|_| String::new());
    for (index, query) in queries.iter().enumerate() {
        let mut explain = db.prepare(&format!("EXPLAIN QUERY PLAN {query}")).unwrap();
        plans[index] = explain
            .query_map(
                params!["changed.ts", "typescript", false, "[]", None::<&str>],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join(" | ");
    }
    times.iter_mut().for_each(|samples| samples.sort_unstable());
    eprintln!(
        "PENDING_BENCH {}",
        serde_json::json!({
            "case": "provider-heavy",
            "repeats": 5,
            "current": {"median_us": times[0][2].as_micros(), "vm_steps": vm_steps[0], "plan": plans[0]},
            "split": {"median_us": times[1][2].as_micros(), "vm_steps": vm_steps[1], "plan": plans[1]}
        })
    );
}

#[test]
fn rust_super_like_specialized_vm_timing_and_plan_benchmark() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE nodes(file_path TEXT,kind TEXT,name TEXT,qualified_name TEXT,visibility TEXT,is_exported INTEGER);
         CREATE TABLE unresolved_refs(id INTEGER PRIMARY KEY,reference_name TEXT,file_path TEXT,language TEXT,reference_kind TEXT,status TEXT);
         CREATE INDEX idx_unresolved_file_path ON unresolved_refs(file_path);
         CREATE INDEX idx_unresolved_name ON unresolved_refs(reference_name);
         CREATE INDEX idx_unresolved_status ON unresolved_refs(status);
         CREATE INDEX idx_nodes_file_path ON nodes(file_path);
         INSERT INTO nodes VALUES('changed.rs','struct','Thing','Thing','public',1);",
    )
    .unwrap();
    for id in 1..=20_000 {
        db.execute(
            "INSERT INTO unresolved_refs VALUES(?1,'unrelated::name','other.rs','rust','calls','pending')",
            [id],
        )
        .unwrap();
    }
    db.execute_batch(
        "INSERT INTO unresolved_refs VALUES(20001,'super::Thing','other.rs','rust','calls','pending');
         INSERT INTO unresolved_refs VALUES(20002,'SUPER::Thing','other.rs','rust','calls','pending');
         INSERT INTO unresolved_refs VALUES(20003,'super::Failed','other.rs','rust','function_ref','failed');",
    )
    .unwrap();
    let queries = [PENDING_SQL, RUST_PENDING_SQL];
    let mut expected: [Option<Vec<i64>>; 2] = [None, None];
    for (index, query) in queries.iter().enumerate() {
        let mut statement = db.prepare(query).unwrap();
        expected[index] = Some(
            statement
                .query_map(
                    params!["changed.rs", "rust", false, "[]", None::<&str>],
                    |row| row.get(0),
                )
                .unwrap()
                .map(Result::unwrap)
                .collect(),
        );
    }
    assert_eq!(expected[0], expected[1]);

    let mut times: [Vec<std::time::Duration>; 2] = [Vec::new(), Vec::new()];
    let mut vm_steps = [0; 2];
    for repetition in 0..5 {
        let order = if repetition % 2 == 0 { [0, 1] } else { [1, 0] };
        for index in order {
            let mut statement = db.prepare(queries[index]).unwrap();
            let started = std::time::Instant::now();
            let ids: Vec<i64> = statement
                .query_map(
                    params!["changed.rs", "rust", false, "[]", None::<&str>],
                    |row| row.get(0),
                )
                .unwrap()
                .map(Result::unwrap)
                .collect();
            times[index].push(started.elapsed());
            vm_steps[index] = statement.get_status(StatementStatus::VmStep);
            assert_eq!(Some(&ids), expected[index].as_ref());
            assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }
    let mut plans: [String; 2] = std::array::from_fn(|_| String::new());
    for (index, query) in queries.iter().enumerate() {
        let mut explain = db.prepare(&format!("EXPLAIN QUERY PLAN {query}")).unwrap();
        plans[index] = explain
            .query_map(
                params!["changed.rs", "rust", false, "[]", None::<&str>],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join(" | ");
    }
    times.iter_mut().for_each(|samples| samples.sort_unstable());
    let like_matches_uppercase: i64 = db
        .query_row(
            "SELECT count(*) FROM unresolved_refs WHERE reference_name LIKE 'super::%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    eprintln!(
        "PENDING_BENCH {}",
        serde_json::json!({
            "case": "rust-super-specialized",
            "repeats": 5,
            "like_matches_uppercase": like_matches_uppercase,
            "current": {"median_us": times[0][2].as_micros(), "vm_steps": vm_steps[0], "plan": plans[0]},
            "rust_specialized": {"median_us": times[1][2].as_micros(), "vm_steps": vm_steps[1], "plan": plans[1]}
        })
    );
    assert!(like_matches_uppercase >= 2);
}

#[test]
fn split_pending_profile_benchmarks_preserve_ordered_ids() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE nodes(file_path TEXT,kind TEXT,name TEXT,qualified_name TEXT,visibility TEXT,is_exported INTEGER);
         CREATE TABLE unresolved_refs(id INTEGER PRIMARY KEY,reference_name TEXT,file_path TEXT,language TEXT,reference_kind TEXT,status TEXT);
         CREATE INDEX idx_unresolved_file_path ON unresolved_refs(file_path);
         CREATE INDEX idx_unresolved_name ON unresolved_refs(reference_name);
         CREATE INDEX idx_unresolved_status ON unresolved_refs(status);
         CREATE INDEX idx_nodes_file_path ON nodes(file_path);
         CREATE INDEX idx_nodes_kind ON nodes(kind,file_path);
         INSERT INTO nodes VALUES('changed.js','function','fresh','fresh','public',1),('importer.js','import','alias','alias',NULL,0);",
    )
    .unwrap();
    for id in 1..=40_000 {
        db.execute(
            "INSERT INTO unresolved_refs VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                id,
                match id % 11 {
                    0 => "fresh",
                    1 => "super::Method",
                    2 => "Provider",
                    _ => "unrelated::name"
                },
                if id % 7 == 0 { "importer.js" } else { "other.js" },
                match id % 4 {
                    0 => "rust",
                    1 => "typescript",
                    2 => "javascript",
                    _ => "csharp"
                },
                match id % 5 {
                    0 => "angular_provider_token",
                    1 => "angular_injects",
                    _ => "calls"
                },
                if id % 6 == 0 { "failed" } else { "pending" }
            ],
        )
        .unwrap();
    }
    for (language, config_changed) in [
        ("rust", false),
        ("javascript", false),
        ("javascript", true),
        ("csharp", false),
    ] {
        let routed = pending_sql_for(language, config_changed, false, true);
        let queries = [PENDING_SQL, routed];
        let mut expected_ids: Option<Vec<i64>> = None;
        for query in queries {
            let mut statement = db.prepare(query).unwrap();
            let ids: Vec<i64> = statement
                .query_map(
                    params!["changed.js", language, config_changed, "[]", None::<&str>],
                    |row| row.get(0),
                )
                .unwrap()
                .map(Result::unwrap)
                .collect();
            if let Some(expected) = &expected_ids {
                assert_eq!(&ids, expected, "{language}, config={config_changed}");
            } else {
                expected_ids = Some(ids);
            }
        }
        let queries = [PENDING_SQL, routed];
        let mut times: [Vec<std::time::Duration>; 2] = [Vec::new(), Vec::new()];
        let mut vm_steps = [0; 2];
        for repetition in 0..5 {
            let order = if repetition % 2 == 0 { [0, 1] } else { [1, 0] };
            for index in order {
                let mut statement = db.prepare(queries[index]).unwrap();
                let started = std::time::Instant::now();
                let ids: Vec<i64> = statement
                    .query_map(
                        params!["changed.js", language, config_changed, "[]", None::<&str>],
                        |row| row.get(0),
                    )
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                times[index].push(started.elapsed());
                vm_steps[index] = statement.get_status(StatementStatus::VmStep);
                assert_eq!(Some(&ids), expected_ids.as_ref());
                assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
            }
        }
        times.iter_mut().for_each(|samples| samples.sort_unstable());
        eprintln!(
            "PENDING_BENCH {}",
            serde_json::json!({
                "case": "language-profile",
                "language": language,
                "config_changed": config_changed,
                "repeats": 5,
                "current": {"median_us": times[0][2].as_micros(), "vm_steps": vm_steps[0]},
                "routed": {"median_us": times[1][2].as_micros(), "vm_steps": vm_steps[1]}
            })
        );
    }
}
