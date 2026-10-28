use super::*;
use rusqlite::Connection;
use super::super::INDEX_BUSY;

#[test]
fn sqlite_busy_and_locked_remain_busy_failures_after_the_query_budget() {
    for (code, cause) in [
        (rusqlite::ffi::SQLITE_BUSY, "database_busy"),
        (rusqlite::ffi::SQLITE_LOCKED, "database_locked"),
    ] {
        let diagnostic = RefCell::new(json!({}));
        let context = OperationContext::default();
        let error = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            Some("private SQLite error detail".into()),
        );
        let failure = failure(
            error,
            &diagnostic,
            &context,
            Instant::now() - Duration::from_secs(6),
        );

        assert_eq!(failure, INDEX_BUSY);
        assert_eq!(diagnostic.borrow()["cause"], cause);
        assert!(!failure.1.contains("private SQLite error detail"));
    }
}

#[test]
fn cancellation_and_deadline_take_priority_over_sqlite_busy() {
    let error = || {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            Some("private SQLite error detail".into()),
        )
    };

    let cancelled = OperationContext::default();
    cancelled.cancel();
    let expected = cancelled.check().unwrap_err();
    let cancelled_failure = failure(
        error(),
        &RefCell::new(json!({})),
        &cancelled,
        Instant::now() - Duration::from_secs(6),
    );
    assert_eq!(cancelled_failure.0, expected.0);

    let timed_out = OperationContext::with_timeout(Duration::ZERO);
    let expected = timed_out.check().unwrap_err();
    let timeout_failure = failure(
        error(),
        &RefCell::new(json!({})),
        &timed_out,
        Instant::now() - Duration::from_secs(6),
    );
    assert_eq!(timeout_failure.0, expected.0);
}

#[cfg(windows)]
#[test]
fn read_open_waits_for_an_exclusive_sqlite_transaction_to_release() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("codegraph.db");
    let blocker = Connection::open(&path).unwrap();
    blocker
        .execute_batch(
            "PRAGMA journal_mode=DELETE;
             CREATE TABLE schema_versions(version INTEGER);
             INSERT INTO schema_versions(version) VALUES(11);
             CREATE TABLE nodes(id INTEGER, qualified_name TEXT);
             CREATE TABLE edges(id INTEGER);
             BEGIN EXCLUSIVE;",
        )
        .unwrap();

    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        blocker.execute_batch("ROLLBACK;").unwrap();
    });
    let diagnostic = RefCell::new(json!({}));
    let context = OperationContext::default();
    let started = Instant::now();
    let opened = open(&path, &context, &diagnostic, started);
    release.join().unwrap();

    let opened = opened.unwrap();
    assert_eq!(opened.version, 11);
    assert!(started.elapsed() >= Duration::from_millis(100));
}

#[test]
fn explore_populates_cross_layer_data_trace_for_t_observacion_empleado() {
    use harness_domain::contracts::ExploreArgs;

    let root = tempfile::tempdir().unwrap();
    let cg_dir = root.path().join(".codegraph");
    std::fs::create_dir_all(&cg_dir).unwrap();
    let db = Connection::open(cg_dir.join("codegraph.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE schema_versions(version INTEGER);
         INSERT INTO schema_versions VALUES(11);
         CREATE TABLE files(path TEXT PRIMARY KEY, content_hash TEXT, errors TEXT);
         CREATE TABLE nodes(id TEXT PRIMARY KEY, name TEXT, kind TEXT, file_path TEXT, start_line INTEGER, qualified_name TEXT);
         CREATE TABLE edges(id INTEGER PRIMARY KEY, source TEXT, target TEXT, kind TEXT);",
    )
    .unwrap();

    std::fs::create_dir_all(root.path().join("Views/Empleado")).unwrap();
    std::fs::create_dir_all(root.path().join("Scripts")).unwrap();
    std::fs::create_dir_all(root.path().join("Models")).unwrap();
    std::fs::create_dir_all(root.path().join("Data")).unwrap();
    std::fs::create_dir_all(root.path().join("Sql")).unwrap();

    let cshtml = "<input id=\"tObservacionEmpleado\" name=\"tObservacionEmpleado\" value=\"@Model.tObservacionEmpleado\" />\n";
    let js = "const data = { tObservacionEmpleado: $('#tObservacionEmpleado').val() };\n";
    let cs_model = "public class EmpleadoModel {\n    public string tObservacionEmpleado { get; set; }\n}\n";
    let cs_repo = "public class EmpleadoRepo {\n    public void Save(EmpleadoModel m) {\n        cmd.Parameters.Add(new SqlParameter(\"@tObservacionEmpleado\", m.tObservacionEmpleado));\n    }\n}\n";
    let sql = "CREATE PROCEDURE dbo.sp_SaveEmpleado\n    @tObservacionEmpleado VARCHAR(500)\nAS\nBEGIN\n    INSERT INTO dbo.Empleado (ObservacionEmpleado) VALUES (@tObservacionEmpleado);\nEND\n";

    std::fs::write(root.path().join("Views/Empleado/Index.cshtml"), cshtml).unwrap();
    std::fs::write(root.path().join("Scripts/empleado.js"), js).unwrap();
    std::fs::write(root.path().join("Models/EmpleadoModel.cs"), cs_model).unwrap();
    std::fs::write(root.path().join("Data/EmpleadoRepo.cs"), cs_repo).unwrap();
    std::fs::write(root.path().join("Sql/sp_SaveEmpleado.sql"), sql).unwrap();

    db.execute(
        "INSERT INTO files VALUES('Models/EmpleadoModel.cs', ?1, NULL)",
        [harness_domain::hash(cs_model.as_bytes())],
    )
    .unwrap();
    db.execute(
        "INSERT INTO nodes VALUES('prop-1', 'tObservacionEmpleado', 'property', 'Models/EmpleadoModel.cs', 2, 'EmpleadoModel.tObservacionEmpleado')",
        [],
    )
    .unwrap();
    drop(db);

    let project = Project::new(root.path()).unwrap();
    let args: ExploreArgs = serde_json::from_value(json!({
        "query": "@tObservacionEmpleado"
    }))
    .unwrap();
    let result = super::super::explore(&project, &args, &OperationContext::default()).unwrap();

    let trace = &result["dataTrace"];
    assert_eq!(trace["identifier"], "tObservacionEmpleado");
    assert_eq!(trace["normalizedIdentifier"], "ObservacionEmpleado");
    assert_eq!(trace["parameterName"], "@tObservacionEmpleado");
    assert_eq!(trace["completeChain"], true);
    assert_eq!(trace["matchedLayerCount"], 5);
    assert_eq!(result["crossLayerTrace"], *trace);
    assert_eq!(result["_detail"]["dataTrace"], *trace);
    assert!(
        result["text"]
            .as_str()
            .unwrap()
            .contains("Form (.cshtml) → JSON/JS (.js) → C# Model/Controller (.cs) → SP Parameter (@...) → SQL Column (.sql)")
    );
}
