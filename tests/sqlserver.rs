use agent_harness::adapters;
use harness_domain::sql::SqlArgs;
use serde_json::{Value, json};

fn setup(config: Value) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".agents")).unwrap();
    std::fs::write(
        root.path().join(".agents/harness.config.json"),
        json!({"sqlServer":config}).to_string(),
    )
    .unwrap();
    root
}
fn call(root: &std::path::Path, request: Value) -> Value {
    let args: SqlArgs = serde_json::from_value(request).unwrap();
    adapters::application(root)
        .unwrap()
        .execute(args.into_request())
}

#[test]
fn status_defaults_to_offline_and_never_calls_missing_credentials_ready() {
    let root = setup(json!({"enabled":true,"connection":{"path":"missing.config"}}));
    for extra in [json!({}), json!({"connect":false})] {
        let mut request = json!({"action":"status","engine":"sqlserver"});
        request
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let result = call(root.path(), request);
        assert_eq!(result["state"], "ok", "{result}");
        assert_eq!(result["data"]["engine"], "sqlserver");
        assert_eq!(result["data"]["status"], "configured_unverified");
        assert_eq!(result["data"]["connected"], false);
    }
    let failed = call(root.path(), json!({"action":"status","connect":true}));
    assert_eq!(failed["state"], "error", "{failed}");
    assert_eq!(failed["data"]["status"], "verification_failed");
    assert_eq!(failed["diagnostics"][0]["code"], "SQL_CONFIG");
}

#[test]
fn disabled_status_has_the_same_core_fields_as_postgres() {
    let root = setup(json!({"enabled":false}));
    let result = call(root.path(), json!({"action":"status"}));
    assert_eq!(result["data"]["configured"], false);
    assert_eq!(result["data"]["connected"], false);
    assert_eq!(result["data"]["status"], "not_configured");
    assert_eq!(result["data"]["engine"], "sqlserver");
}

#[test]
fn config_limits_are_bounded_even_for_extreme_values() {
    let root = setup(
        json!({"enabled":true,"maxRows":u64::MAX,"maxChars":u64::MAX,"timeoutSeconds":u64::MAX}),
    );
    let result = call(root.path(), json!({"action":"status"}));
    assert_eq!(result["data"]["maxRows"], 200);
    assert_eq!(result["data"]["maxChars"], 32000);
    assert_eq!(result["data"]["timeoutSeconds"], 120);
}

#[test]
fn invalid_parameters_and_sections_fail_before_network_access() {
    let root = setup(json!({"enabled":true,"connection":{"path":"missing.config"}}));
    for parameters in [
        json!({}),
        json!({"id":{}}),
        json!({"id":1,"unused":2}),
        json!({"id":1,"@ID":2}),
    ] {
        let result = call(
            root.path(),
            json!({"action":"query","sql":"SELECT @id","parameters":parameters}),
        );
        assert_eq!(
            result["diagnostics"][0]["code"], "SQL_PARAMETERS",
            "{result}"
        );
    }
    let result = call(
        root.path(),
        json!({"action":"describe","name":"dbo.test","sections":["indexes"]}),
    );
    assert_eq!(result["diagnostics"][0]["code"], "SQL_DESCRIBE_SECTION");
}

#[test]
fn stalled_handshake_obeys_parent_deadline_and_closes_connection() {
    use std::{
        io::Read,
        net::TcpListener,
        time::{Duration, Instant},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let root = setup(
        json!({"enabled":true,"timeoutSeconds":15,"connection":{"path":"Web.config","name":"fixture"}}),
    );
    std::fs::write(root.path().join("Web.config"), format!("<configuration><add name=\"fixture\" connectionString=\"Server=127.0.0.1,{port};User ID=fixture;Password=not-a-real-secret\" /></configuration>")).unwrap();
    // Deliberately not a database: accept TCP but never complete the TDS handshake.
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    // Windows accepted sockets inherit the listener's nonblocking mode.
                    socket.set_nonblocking(false).unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut buffer = [0; 4096];
                    loop {
                        match socket.read(&mut buffer) {
                            Ok(0) => return true,
                            Ok(_) => {}
                            Err(_) => return false,
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                _ => return false,
            }
        }
    });
    let app = adapters::application(root.path()).unwrap();
    let args: SqlArgs = serde_json::from_value(json!({"action":"status","connect":true})).unwrap();
    let started = Instant::now();
    let result = app.execute_with_context(
        args.into_request(),
        harness_domain::execution::OperationContext::with_timeout(Duration::from_millis(300)),
    );
    assert_eq!(result["state"], "timeout", "{result}");
    assert_eq!(result["diagnostics"][0]["code"], "TIMEOUT", "{result}");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        server.join().unwrap(),
        "timed-out TDS socket was not closed"
    );
    assert!(!result.to_string().contains("not-a-real-secret"));
}

#[test]
#[ignore = "requires an explicitly provided dedicated HARNESS_SQL_TEST_DSN fixture"]
fn sqlserver_live_binds_scalars_and_bounds_streamed_results() {
    let dsn = std::env::var("HARNESS_SQL_TEST_DSN").expect("dedicated fixture DSN required");
    let root = setup(
        json!({"enabled":true,"maxRows":2,"maxChars":1000,"connection":{"path":"Web.config","name":"fixture"}}),
    );
    let escaped = dsn
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    std::fs::write(root.path().join("Web.config"), format!("<configuration><connectionStrings><add name=\"fixture\" connectionString=\"{escaped}\" /></connectionStrings></configuration>")).unwrap();
    let result = call(
        root.path(),
        json!({"action":"query","sql":"SELECT @id AS id, @label AS label, @flag AS flag, CAST(@empty AS nvarchar(10)) AS empty_value, CAST(@amount AS decimal(12,2)) AS amount", "parameters":{"id":42,"label":"'; DROP TABLE example;--","flag":true,"empty":null,"amount":12.25}}),
    );
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["rows"][0]["id"], 42);
    assert_eq!(
        result["data"]["rows"][0]["label"],
        "'; DROP TABLE example;--"
    );
    assert_eq!(result["data"]["rows"][0]["flag"], true);
    assert_eq!(result["data"]["rows"][0]["empty_value"], Value::Null);
    assert_eq!(result["data"]["readOnlyVerified"], false);
    let result = call(
        root.path(),
        json!({"action":"query","sql":"SELECT x.n FROM (VALUES (1),(2),(3),(4)) x(n)"}),
    );
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["rowCount"], 2);
    assert_eq!(result["omitted"]["truncated"], true);
    let result = call(
        root.path(),
        json!({"action":"query","sql":"SELECT REPLICATE(CAST('x' AS nvarchar(max)),10000) AS value"}),
    );
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["rowCount"], 0);
    assert_eq!(result["omitted"]["truncated"], true);
    assert!(result["data"]["rows"].to_string().len() <= 1000);
}
