#![cfg(windows)]
use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{DoctorArgs, LogsArgs, ReadArgs, Request};
use serde_json::Value;
fn read(app: &NativeApplication) -> Value {
    app.execute(Request::Read(ReadArgs {
        external_scope: None,
        path: "good.txt".into(),
        start: 1,
        end: 1,
        compact: false,
        line_numbers: false,
        session: None,
    }))
}
fn logs(app: &NativeApplication, id: &str) -> Value {
    app.execute(Request::Logs(LogsArgs {
        limit: None,
        level: None,
        tool: None,
        slow_only: None,
        request_id: Some(id.into()),
        after: None,
        session: None,
    }))
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("good.txt"), "fixture").unwrap();
    root
}

#[test]
fn logs_survive_restart_with_history_flags_but_metrics_do_not() {
    let root = fixture();
    let app = adapters::application_with_logging(root.path(), true).unwrap();
    let result = read(&app);
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["logPersistence"]["saved"], true, "{result}");
    let id = result["requestId"].as_str().unwrap();
    assert_eq!(logs(&app, id)["data"]["events"][0]["historical"], false);
    drop(app);
    let app = adapters::application_with_logging(root.path(), true).unwrap();
    let restored = logs(&app, id);
    assert_eq!(restored["logPersistence"]["saved"], true, "{restored}");
    assert_eq!(
        restored["data"]["events"][0]["historical"], true,
        "{restored}"
    );
    let doctor = app.execute(Request::Doctor(DoctorArgs {
        versions: Some(false),
        connection: None,
        session: None,
    }));
    assert!(
        doctor["data"]["metrics"]["byTool"]
            .get("harness_read")
            .is_none()
    );
    assert_eq!(doctor["data"]["logging"]["persistent"], true);
}

#[test]
fn rotation_is_bounded_and_second_writer_does_not_change_tool_outcome() {
    let root = fixture();
    let app = adapters::application_with_logging(root.path(), true).unwrap();
    for _ in 0..15 {
        assert_eq!(read(&app)["logPersistence"]["saved"], true);
    }
    let directory = root.path().join(".agents/harness/native-logs");
    assert!(std::fs::rename(&directory, root.path().join("moved-native-logs")).is_err());
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 3);
    for name in ["current.json", "previous.json"] {
        let bytes = std::fs::read(directory.join(name)).unwrap();
        assert!(bytes.len() <= harness_domain::log_archive::MAX_BYTES);
        harness_domain::log_archive::validate(&serde_json::from_slice(&bytes).unwrap()).unwrap();
    }
    let other = adapters::application_with_logging(root.path(), true).unwrap();
    let blocked = read(&other);
    assert_eq!(blocked["state"], "ok", "{blocked}");
    assert_eq!(blocked["loggingWarning"]["code"], "LOG_BUSY");
    assert_eq!(blocked["logPersistence"]["saved"], false);
    drop(other);
    drop(app);
    let restarted = adapters::application_with_logging(root.path(), true).unwrap();
    assert_eq!(read(&restarted)["logPersistence"]["saved"], true);
}

#[test]
fn unknown_archives_and_hardlinks_are_never_overwritten_or_exposed() {
    let root = fixture();
    let directory = root.path().join(".agents/harness/native-logs");
    std::fs::create_dir_all(&directory).unwrap();
    let secret = b"unrecognized existing file - do not expose";
    std::fs::write(directory.join("current.json"), secret).unwrap();
    let app = adapters::application_with_logging(root.path(), true).unwrap();
    let result = read(&app);
    assert_eq!(result["state"], "ok");
    assert_eq!(result["loggingWarning"]["code"], "LOG_FORMAT", "{result}");
    assert_eq!(
        std::fs::read(directory.join("current.json")).unwrap(),
        secret
    );
    assert!(!result.to_string().contains("unrecognized existing"));
    drop(app);
    std::fs::remove_file(directory.join("current.json")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("outside.json");
    std::fs::write(&target, secret).unwrap();
    std::fs::hard_link(&target, directory.join("current.json")).unwrap();
    let app = adapters::application_with_logging(root.path(), true).unwrap();
    let result = read(&app);
    assert_eq!(result["loggingWarning"]["code"], "LOG_PATH", "{result}");
    assert_eq!(std::fs::read(&target).unwrap(), secret);
}

#[test]
fn a_valid_stage_recovers_after_an_interrupted_publication() {
    let root = fixture();
    let app = adapters::application_with_logging(root.path(), true).unwrap();
    let first = read(&app);
    assert_eq!(first["logPersistence"]["saved"], true, "{first}");
    drop(app);
    let directory = root.path().join(".agents/harness/native-logs");
    std::fs::copy(directory.join("current.json"), directory.join("next.json")).unwrap();
    std::fs::rename(
        directory.join("current.json"),
        directory.join("previous.json"),
    )
    .unwrap();
    let recovered = adapters::application_with_logging(root.path(), true).unwrap();
    let result = logs(&recovered, first["requestId"].as_str().unwrap());
    assert_eq!(result["data"]["events"][0]["historical"], true, "{result}");
    assert_eq!(result["logPersistence"]["saved"], true);
    assert!(!directory.join("next.json").exists());
    drop(recovered);
    let current = std::fs::read(directory.join("current.json")).unwrap();
    std::fs::write(directory.join("next.json"), &current).unwrap();
    let stale = adapters::application_with_logging(root.path(), true).unwrap();
    let result = read(&stale);
    assert_eq!(result["state"], "ok");
    assert_eq!(result["loggingWarning"]["code"], "LOG_FORMAT");
    assert_eq!(
        std::fs::read(directory.join("current.json")).unwrap(),
        current
    );
}
