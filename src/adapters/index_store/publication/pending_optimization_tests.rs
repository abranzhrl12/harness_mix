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
            ('owner','changed.js','javascript','function','owner','owner',NULL,0);
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
            (21,'owner','ProviderFailed','angular_provider_token',21,1,NULL,'failed-provider.ts','typescript','failed','');",
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
    let mut ids: Vec<_> = rows.map(Result::unwrap).collect();
    ids.sort_unstable();
    ids
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

fn assert_equivalent(
    db: &Connection,
    path: &str,
    language: &str,
    config_changed: bool,
    attempted_names: &str,
    import_bindings: Option<&str>,
) {
    let legacy_query = effective_legacy_query(language, config_changed);
    let legacy = query_ids(
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
    assert_eq!(
        current, legacy,
        "{language}, config={config_changed}, names={attempted_names}, bindings={import_bindings:?}"
    );
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
            "INSERT INTO unresolved_refs VALUES(?1,'same','changed.js','javascript','calls','pending')",
            [id],
        )
        .unwrap();
    }
    let old_query = effective_legacy_query("javascript", false);
    let old = query_ids(
        &db,
        &old_query,
        "changed.js",
        "javascript",
        false,
        "[]",
        None,
    );
    let new = query_ids(
        &db,
        PENDING_SQL,
        "changed.js",
        "javascript",
        false,
        "[]",
        None,
    );
    assert_eq!(old.len(), 20_001);
    assert_eq!(new.len(), 20_001);
    assert_eq!(old, new);
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
    for query in [LEGACY_PENDING_SQL, PENDING_SQL] {
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
    assert_eq!(measurements[0].0, measurements[1].0);
    eprintln!(
        "pending VM steps: legacy={}, current={}",
        measurements[0].1, measurements[1].1
    );
    assert!(
        measurements[1].1 * 2 < measurements[0].1,
        "expected at least 50% fewer VM steps: legacy={}, current={}",
        measurements[0].1,
        measurements[1].1
    );
}
