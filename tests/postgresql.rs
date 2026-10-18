use agent_harness::adapters;
use harness_domain::{contracts::Request, sql::SqlArgs};
use serde_json::{Value, json};

fn setup(pg: Value, ms: bool) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".agents")).unwrap();
    std::fs::write(
        root.path().join(".agents/harness.config.json"),
        json!({"postgresql":pg,"sqlServer":{"enabled":ms}}).to_string(),
    )
    .unwrap();
    root
}
fn call(root: &std::path::Path, value: Value) -> Value {
    let args: SqlArgs = serde_json::from_value(value).unwrap();
    adapters::application(root)
        .unwrap()
        .execute(args.into_request())
}

#[test]
fn configured_status_does_not_read_credentials_or_open_a_connection() {
    let root = setup(
        json!({"enabled":true,"connection":{"source":"environment","name":"HARNESS_TEST_MISSING_PG_DSN"}}),
        false,
    );
    let result = call(root.path(), json!({"action":"status","connect":false}));
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["engine"], "postgresql");
    assert_eq!(result["data"]["connected"], false);
    assert_eq!(result["data"]["status"], "configured_unverified");
    let failed = call(
        root.path(),
        json!({"action":"status","engine":"postgresql","connect":true}),
    );
    assert_eq!(failed["state"], "error");
    assert_eq!(failed["diagnostics"][0]["code"], "PG_CREDENTIALS");
}

#[test]
fn engine_is_required_when_both_databases_are_enabled() {
    let root = setup(json!({"enabled":true}), true);
    let result = call(root.path(), json!({"action":"status","connect":false}));
    assert_eq!(result["diagnostics"][0]["code"], "SQL_ENGINE_REQUIRED");
    assert_eq!(
        call(
            root.path(),
            json!({"action":"status","engine":"sqlserver","connect":false})
        )["state"],
        "ok"
    );
}

#[test]
fn postgres_rejects_unapproved_schemas_sections_and_arbitrary_queries() {
    let root = setup(json!({"enabled":true}), false);
    for (request, code) in [
        (
            json!({"action":"search","query":"users","schema":"private"}),
            "PG_SCHEMA",
        ),
        (
            json!({"action":"describe","name":"public.users","sections":["unknown"]}),
            "PG_DESCRIBE_SECTION",
        ),
        (
            json!({"action":"query","sql":"DELETE FROM users"}),
            "PG_QUERY_UNAVAILABLE",
        ),
    ] {
        assert_eq!(call(root.path(), request)["diagnostics"][0]["code"], code);
    }
}

#[test]
fn invalid_config_is_reported_without_returning_its_contents() {
    let root = setup(
        json!({"enabled":true,"maxRows":0,"password":"do-not-return-this"}),
        false,
    );
    let result = call(
        root.path(),
        json!({"action":"status","engine":"postgresql","connect":false}),
    );
    assert_eq!(result["diagnostics"][0]["code"], "PG_CONFIG");
    assert!(!result.to_string().contains("do-not-return-this"));
}

#[test]
fn postgres_setup_is_actionable_without_credentials_or_network() {
    let root = setup(json!({"enabled":false}), false);
    let result = call(
        root.path(),
        json!({"action":"status","engine":"postgresql"}),
    );
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["setup"]["configSection"], "postgresql");
    assert_eq!(
        result["data"]["setup"]["mergeExample"]["postgresql"]["tls"]["mode"],
        "disable"
    );
    assert_eq!(result["diagnostics"][0]["code"], "SQL_NOT_CONFIGURED");
    let root = setup(json!({"enabled":true,"schemaCacheSeconds":301}), false);
    assert_eq!(
        call(
            root.path(),
            json!({"action":"status","engine":"postgresql"})
        )["diagnostics"][0]["code"],
        "PG_CONFIG"
    );
    let root = setup(
        json!({"enabled":true,"schemaCacheSeconds":60,
        "connection":{"source":"environment","name":"HARNESS_TEST_MISSING_PG_DSN"}}),
        false,
    );
    let result = call(
        root.path(),
        json!({"action":"status","engine":"postgresql"}),
    );
    assert_eq!(result["data"]["schemaCacheEnabled"], true);
    assert_eq!(
        result["data"]["connectionEnvironmentVariable"],
        "HARNESS_TEST_MISSING_PG_DSN"
    );
    assert_eq!(result["data"]["workflow"]["parallelTool"], "harness_batch");
}

#[test]
#[ignore = "dedicated fixture/admin only; creates a unique table to test cached metadata, DDL, revocation and refresh"]
fn postgres_live_schema_cache_coalesces_batch_and_refreshes_safely() {
    let name = format!("harness_cache_{}", uuid::Uuid::new_v4().simple());
    admin(&format!(
        "CREATE TABLE public.{name}(id integer); GRANT SELECT ON public.{name} TO harness_reader"
    ));
    let root = setup(
        json!({"enabled":true,"schemaCacheSeconds":60,
        "tls":{"mode":"disable-loopback"},"connection":{"source":"environment","name":"HARNESS_PG_TEST_LOOPBACK_DSN"}}),
        false,
    );
    let request =
        json!({"action":"describe","engine":"postgresql","name":format!("public.{name}")});
    let app = adapters::application(root.path()).unwrap();
    let batch = app.execute(Request::Batch(serde_json::from_value(json!({"jobs":
        (0..4).map(|_| json!({"tool":"harness_sql","arguments":request.clone()})).collect::<Vec<_>>()
    })).unwrap()));
    assert_eq!(batch["state"], "ok", "{batch}");
    let results = batch["data"]["results"].as_array().unwrap();
    assert_eq!(
        results
            .iter()
            .filter(|r| r["data"]["schemaCache"]["hit"] == true)
            .count(),
        3,
        "{batch}"
    );
    let hit = call(root.path(), request.clone());
    assert_eq!(hit["data"]["schemaCache"]["hit"], true, "{hit}");
    assert_eq!(hit["data"]["protocolExchanges"], 0);
    assert_eq!(hit["data"]["databaseAccess"], false);
    assert_eq!(hit["data"]["schemaCache"]["permissionsRevalidated"], false);
    assert!(hit["data"].get("timings").is_none());
    admin(&format!("ALTER TABLE public.{name} ADD COLUMN label text"));
    assert_eq!(call(root.path(), request.clone())["data"]["count"], 1);
    let mut refresh = request.clone();
    refresh["refresh"] = json!(true);
    let fresh = call(root.path(), refresh.clone());
    assert_eq!(fresh["data"]["count"], 2, "{fresh}");
    assert_eq!(fresh["data"]["protocolExchanges"], 1);
    assert_eq!(fresh["data"]["transactionCompletion"], "ready_for_query");
    let status = call(root.path(), json!({"action":"status","connect":true}));
    assert_eq!(status["data"]["server"][0]["read_only"], "on", "{status}");
    assert_ne!(fresh["data"]["schemaHash"], hit["data"]["schemaHash"]);
    admin(&format!(
        "REVOKE SELECT ON public.{name} FROM harness_reader"
    ));
    // Opt-in snapshots explicitly retain old visibility until TTL/refresh, never claim live permission checks.
    assert_eq!(call(root.path(), request.clone())["data"]["count"], 2);
    let revoked = call(root.path(), refresh);
    assert_eq!(revoked["data"]["count"], 0, "{revoked}");
    assert_eq!(
        call(root.path(), request)["data"]["schemaCache"]["hit"],
        false
    );
    admin(&format!("DROP TABLE public.{name}"));
}

#[test]
#[ignore = "dedicated PostgreSQL fixture; validates TTL expiry, config rotation and no cache of truncated schemas"]
fn postgres_live_schema_cache_expires_and_rotates() {
    let config = |ttl, rows| {
        json!({"enabled":true,"schemaCacheSeconds":ttl,"maxRows":rows,
        "tls":{"mode":"disable-loopback"},"connection":{"source":"environment","name":"HARNESS_PG_TEST_LOOPBACK_DSN"}})
    };
    let root = setup(config(1, 50), false);
    let request = json!({"action":"describe","name":"public.harness_catalog_fixture"});
    assert_eq!(
        call(root.path(), request.clone())["data"]["schemaCache"]["hit"],
        false
    );
    assert_eq!(
        call(root.path(), request.clone())["data"]["schemaCache"]["hit"],
        true
    );
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert_eq!(
        call(root.path(), request.clone())["data"]["schemaCache"]["hit"],
        false
    );
    std::fs::write(
        root.path().join(".agents/harness.config.json"),
        json!({"postgresql":config(60,1)}).to_string(),
    )
    .unwrap();
    for _ in 0..2 {
        let partial = call(root.path(), request.clone());
        assert_eq!(partial["state"], "ok", "{partial}");
        assert_eq!(partial["omitted"]["truncated"], true);
        assert_eq!(partial["data"]["schemaCache"]["hit"], false);
    }
}

#[test]
#[ignore = "dedicated fixture; HARNESS_PG_TEST_POLICY_DSN intentionally requests unsafe startup defaults"]
fn postgres_live_startup_policy_overrides_unsafe_dsn_defaults() {
    let root = setup(
        json!({"enabled":true,"tls":{"mode":"disable-loopback"},
        "connection":{"source":"environment","name":"HARNESS_PG_TEST_POLICY_DSN"}}),
        false,
    );
    let result = call(root.path(), json!({"action":"status","connect":true}));
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["server"][0]["read_only"], "on", "{result}");
    let described = call(
        root.path(),
        json!({"action":"describe","name":"public.harness_catalog_fixture"}),
    );
    assert_eq!(described["state"], "ok", "{described}");
    assert_eq!(described["data"]["transactionMode"], "implicit_read_only");
}

#[test]
#[ignore = "requires a dedicated PostgreSQL catalog fixture and HARNESS_PG_TEST_DSN"]
fn postgres_live_catalog() {
    // The dedicated fixture must contain public.harness_catalog_fixture (id, label).
    let root = setup(
        json!({"enabled":true,"tls":{"mode":"disable-loopback"},
        "connection":{"source":"environment","name":"HARNESS_PG_TEST_DSN"}}),
        false,
    );
    let status = call(root.path(), json!({"action":"status","connect":true}));
    assert_eq!(status["data"]["connected"], true, "{status}");
    let search = call(
        root.path(),
        json!({"action":"search","query":"harness_catalog_fixture","objectType":"table"}),
    );
    assert_eq!(search["state"], "ok", "{search}");
    assert!(search["data"]["count"].as_u64().unwrap() > 0, "{search}");
    let describe = call(
        root.path(),
        json!({"action":"describe","name":"public.harness_catalog_fixture"}),
    );
    assert_eq!(describe["state"], "ok", "{describe}");
    assert!(
        describe["data"]["count"].as_u64().unwrap() >= 2,
        "{describe}"
    );
    let columns = call(
        root.path(),
        json!({"action":"search","query":"label","objectType":"column"}),
    );
    assert_eq!(columns["state"], "ok", "{columns}");
    assert!(columns["data"]["count"].as_u64().unwrap() > 0, "{columns}");
    let routines = call(
        root.path(),
        json!({"action":"search","query":"harness_catalog_routine","objectType":"function"}),
    );
    assert_eq!(routines["state"], "ok", "{routines}");
    assert_eq!(routines["data"]["count"], 2, "{routines}");
    let invisible = call(
        root.path(),
        json!({"action":"search","query":"harness_invisible","objectType":"table"}),
    );
    assert_eq!(invisible["data"]["count"], 0, "{invisible}");
    let limited = setup(
        json!({"enabled":true,"maxRows":1,"tls":{"mode":"disable-loopback"},
        "connection":{"source":"environment","name":"HARNESS_PG_TEST_DSN"}}),
        false,
    );
    let result = call(
        limited.path(),
        json!({"action":"describe","name":"public.harness_catalog_fixture"}),
    );
    assert_eq!(result["data"]["count"], 1, "{result}");
    assert_eq!(result["omitted"]["truncated"], true, "{result}");
    let app = adapters::application(root.path()).unwrap();
    let batch = app.execute(Request::Batch(serde_json::from_value(json!({"jobs":[{"tool":"harness_sql","arguments":{"action":"status","engine":"postgresql","connect":false}}]})).unwrap()));
    assert_eq!(batch["state"], "ok", "{batch}");
}

#[test]
#[ignore = "requires dedicated PostgreSQL TLS fixture, HARNESS_PG_TEST_DSN and HARNESS_PG_TEST_CA"]
fn postgres_live_tls_catalog_and_certificate_rejection() {
    let config = json!({"enabled":true,"connection":{"source":"environment","name":"HARNESS_PG_TEST_DSN"},
        "tls":{"mode":"verify-full","caFile":"pg-ca.pem"}});
    let root = setup(config, false);
    std::fs::copy(
        std::env::var_os("HARNESS_PG_TEST_CA").unwrap(),
        root.path().join("pg-ca.pem"),
    )
    .unwrap();
    let result = call(root.path(), json!({"action":"status","connect":true}));
    assert_eq!(result["data"]["connected"], true, "{result}");
    let reused = call(root.path(), json!({"action":"status","connect":true}));
    assert_eq!(reused["data"]["connectionReused"], true, "{reused}");
    std::fs::write(root.path().join("pg-ca.pem"), "invalid changed CA").unwrap();
    let changed_ca = call(root.path(), json!({"action":"status","connect":true}));
    assert_eq!(changed_ca["state"], "error", "{changed_ca}");
    let untrusted = setup(
        json!({"enabled":true,"tls":{"mode":"verify-full"},"connection":{"source":"environment","name":"HARNESS_PG_TEST_DSN"}}),
        false,
    );
    let failed = call(untrusted.path(), json!({"action":"status","connect":true}));
    assert_eq!(failed["state"], "error", "{failed}");
    let mismatch = setup(
        json!({"enabled":true,"connection":{"source":"environment","name":"HARNESS_PG_TEST_BAD_HOST_DSN"},
        "tls":{"mode":"verify-full","caFile":"pg-ca.pem"}}),
        false,
    );
    std::fs::copy(
        std::env::var_os("HARNESS_PG_TEST_CA").unwrap(),
        mismatch.path().join("pg-ca.pem"),
    )
    .unwrap();
    let failed = call(mismatch.path(), json!({"action":"status","connect":true}));
    assert_eq!(failed["state"], "error", "{failed}");
}

fn admin(sql: &str) -> Vec<tokio_postgres::SimpleQueryMessage> {
    let dsn = std::env::var("HARNESS_PG_TEST_ADMIN_DSN").expect("dedicated test administrator DSN");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let (client, connection) = tokio_postgres::connect(&dsn, tokio_postgres::NoTls)
                .await
                .unwrap();
            let worker = tokio::spawn(connection);
            let result = client.simple_query(sql).await.unwrap();
            drop(client);
            worker.abort();
            let _ = worker.await;
            result
        })
}

#[test]
#[ignore = "dedicated local harness_reader fixture; HARNESS_PG_TEST_ADMIN_DSN; run ignored tests serially"]
fn postgres_live_pool_reuses_statements_but_reads_current_schema_and_permissions() {
    let reader: tokio_postgres::Config = std::env::var("HARNESS_PG_TEST_DSN")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(reader.get_user(), Some("harness_reader"));
    let name = format!("harness_pool_{}", uuid::Uuid::new_v4().simple());
    admin(&format!(
        "CREATE TABLE public.{name}(id integer); GRANT SELECT ON public.{name} TO harness_reader"
    ));
    let root = setup(
        json!({"enabled":true,"tls":{"mode":"disable-loopback"},
        "connection":{"source":"environment","name":"HARNESS_PG_TEST_DSN"}}),
        false,
    );
    let request = json!({"action":"describe","name":format!("public.{name}")});
    let first = call(root.path(), request.clone());
    let second = call(root.path(), request.clone());
    assert_eq!(first["state"], "ok", "{first}");
    assert_eq!(first["data"]["connectionReused"], false);
    assert_eq!(second["data"]["connectionReused"], true, "{second}");
    assert_eq!(second["data"]["statementReused"], true, "{second}");
    assert_eq!(first["data"]["results"], second["data"]["results"]);
    admin(&format!("ALTER TABLE public.{name} ADD COLUMN label text"));
    let changed = call(root.path(), request.clone());
    assert_eq!(changed["data"]["count"], 2, "{changed}");
    admin(&format!(
        "REVOKE SELECT ON public.{name} FROM harness_reader"
    ));
    let revoked = call(root.path(), request.clone());
    assert_eq!(revoked["data"]["connectionReused"], true, "{revoked}");
    assert_eq!(revoked["data"]["count"], 0, "{revoked}");
    admin(&format!("GRANT SELECT ON public.{name} TO harness_reader"));
    let config = json!({"postgresql":{"enabled":true,"maxRows":1,"tls":{"mode":"disable-loopback"},
        "connection":{"source":"environment","name":"HARNESS_PG_TEST_DSN"}}});
    std::fs::write(
        root.path().join(".agents/harness.config.json"),
        config.to_string(),
    )
    .unwrap();
    let rotated = call(root.path(), request);
    assert_eq!(rotated["data"]["connectionReused"], false, "{rotated}");
    assert_eq!(rotated["data"]["count"], 1, "{rotated}");
    admin(&format!("DROP TABLE public.{name}"));
}

#[test]
#[ignore = "dedicated PostgreSQL fixture/admin; checks process session bound with eight callers"]
fn postgres_live_pool_bounds_parallel_sessions() {
    let root = setup(
        json!({"enabled":true,"tls":{"mode":"disable-loopback"},
        "connection":{"source":"environment","name":"HARNESS_PG_TEST_DSN"}}),
        false,
    );
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let root = root.path().to_owned();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                call(
                    &root,
                    json!({"action":"describe","name":"public.harness_catalog_fixture"}),
                )
            })
        })
        .collect();
    for handle in handles {
        let result = handle.join().unwrap();
        assert_eq!(result["state"], "ok", "{result}");
    }
    let count = admin(&format!(
        "SELECT count(*)::text FROM pg_stat_activity WHERE application_name='agent-harness:{}'",
        std::process::id()
    ));
    let count: usize = count
        .iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => row.get(0),
            _ => None,
        })
        .unwrap()
        .parse()
        .unwrap();
    assert!(count <= 4, "physical sessions: {count}");
}

#[test]
#[ignore = "dedicated isolated admin cluster only; temporarily locks pg_attribute to exercise timeout/discard"]
fn postgres_live_pool_discards_a_session_after_timeout() {
    let root = setup(
        json!({"enabled":true,"timeoutSeconds":1,"tls":{"mode":"disable-loopback"},
        "connection":{"source":"environment","name":"HARNESS_PG_TEST_LOOPBACK_DSN"}}),
        false,
    );
    let request = json!({"action":"describe","name":"public.harness_catalog_fixture"});
    let initial = call(root.path(), request.clone());
    assert_eq!(initial["state"], "ok", "{initial}");
    let (ready, wait) = std::sync::mpsc::sync_channel(1);
    let lock = std::thread::spawn(move || {
        let dsn = std::env::var("HARNESS_PG_TEST_ADMIN_DSN").unwrap();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (client, conn) = tokio_postgres::connect(&dsn, tokio_postgres::NoTls)
                    .await
                    .unwrap();
                let worker = tokio::spawn(conn);
                client
                    .batch_execute(
                        "BEGIN; LOCK TABLE pg_catalog.pg_attribute IN ACCESS EXCLUSIVE MODE",
                    )
                    .await
                    .unwrap();
                ready.send(()).unwrap();
                client
                    .batch_execute("SELECT pg_sleep(3); ROLLBACK")
                    .await
                    .unwrap();
                drop(client);
                worker.abort();
                let _ = worker.await;
            });
    });
    wait.recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let failed = call(root.path(), request.clone());
    assert!(
        failed["state"] == "timeout" || failed["state"] == "error",
        "{failed}"
    );
    assert_eq!(failed["data"]["connectionDiscarded"], true, "{failed}");
    lock.join().unwrap();
    let recovered = call(root.path(), request);
    assert_eq!(recovered["state"], "ok", "{recovered}");
    assert_eq!(recovered["data"]["connectionReused"], false, "{recovered}");
}
