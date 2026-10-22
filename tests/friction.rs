#![cfg(windows)]
use agent_harness::adapters;
use harness_domain::contracts::{LogsArgs, ReplaceArgs, Request, VerifyArgs, WriteArgs};
use serde_json::{Value, json};

fn replace(app: &adapters::NativeApplication, value: Value) -> Value {
    app.execute(Request::Replace(
        serde_json::from_value::<ReplaceArgs>(value).unwrap(),
    ))
}

#[test]
fn hashless_edits_require_unique_text_and_preserve_explicit_conflict_checks() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let path = root.path().join("sample.txt");
    std::fs::write(&path, "one\r\ntwo\r\n").unwrap();
    let result = replace(
        &app,
        json!({"path":"sample.txt","oldText":"one\ntwo","newText":"three"}),
    );
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["verified"], true);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "three\r\n");
    let stale = replace(
        &app,
        json!({"path":"sample.txt","hash":"0".repeat(64),"oldText":"three","newText":"four"}),
    );
    assert_eq!(stale["diagnostics"][0]["code"], "STALE");
    std::fs::write(&path, "aa aa").unwrap();
    for value in [
        json!({"path":"sample.txt","oldText":"aa","newText":"bb"}),
        json!({"path":"sample.txt","oldText":"missing","newText":"bb"}),
        json!({"path":"sample.txt","chunks":[{"oldText":"aa aa","newText":"bb"},{"oldText":"aa","newText":"cc"}]}),
    ] {
        assert_eq!(replace(&app, value)["state"], "error");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "aa aa");
    }
}

#[test]
fn native_creation_validates_syntax_and_never_overwrites() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let write = |path: &str, content: &str| {
        app.execute(Request::Write(WriteArgs {
            bom: None,
            create_parents: false,
            allow_unvalidated_text: false,
            expected_hash: None,
            overwrite: false,
            path: path.into(),
            content: content.into(),
            session: None,
        }))
    };
    let created = write("new.rs", "pub fn created() {}\n");
    assert_eq!(created["state"], "ok", "{created}");
    assert_eq!(created["data"]["created"], true);
    assert_eq!(created["data"]["verified"], true);
    assert_eq!(
        write("new.rs", "pub fn replaced() {}\n")["diagnostics"][0]["code"],
        "WRITE_EXISTS"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("new.rs")).unwrap(),
        "pub fn created() {}\n"
    );
    assert_eq!(
        write("invalid.rs", "fn {")["diagnostics"][0]["code"],
        "SYNTAX_ERROR"
    );
    assert!(!root.path().join("invalid.rs").exists());
    for path in [
        "../outside.txt",
        "AGENTS.md",
        ".git/new.txt",
        ".codegraph/new.txt",
        ".agents/new.txt",
        "missing/new.txt",
        ".env",
    ] {
        assert_eq!(write(path, "text")["state"], "error", "{path}");
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn package_check_skips_broken_siblings_and_logs_command_and_error_tails() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("Cargo.toml"),
        "[workspace]\nmembers=['good','broken']\nresolver='3'\n",
    )
    .unwrap();
    for name in ["good", "broken"] {
        std::fs::create_dir_all(root.path().join(name).join("src")).unwrap();
        std::fs::write(
            root.path().join(name).join("Cargo.toml"),
            format!("[package]\nname='{name}'\nversion='0.1.0'\nedition='2024'\n"),
        )
        .unwrap();
    }
    std::fs::write(root.path().join("good/src/lib.rs"), "pub fn works() {}\n").unwrap();
    std::fs::write(
        root.path().join("broken/src/lib.rs"),
        "compile_error!(\"broken sibling\");\n",
    )
    .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let check = |package: &str| {
        app.execute(Request::Verify(VerifyArgs {
            project: None,
            configuration: None,
            language: "rust".into(),
            package: Some(package.into()),
            scope: None,
            action: None,
            engine: None,
            warnings: None,
            session: None,
        }))
    };
    let good = check("good");
    assert_eq!(good["state"], "ok", "{good}");
    assert_eq!(good["data"]["action"], "check");
    assert_eq!(good["data"]["command"]["arguments"][0], "check");
    let broken = check("broken");
    assert_eq!(broken["state"], "error", "{broken}");
    assert!(
        broken["data"]["stderrTail"]
            .as_str()
            .unwrap()
            .contains("broken sibling")
    );
    let logs = app.execute(Request::Logs(LogsArgs {
        limit: Some(50),
        level: None,
        tool: Some("harness_verify".into()),
        slow_only: None,
        request_id: Some(broken["requestId"].as_str().unwrap().into()),
        after: None,
        session: None,
    }));
    assert_eq!(
        logs["data"]["events"][0]["context"]["command"]["program"],
        "cargo"
    );
    assert!(
        logs["data"]["events"][0]["context"]["stderrTail"]
            .as_str()
            .unwrap()
            .contains("broken sibling")
    );
    for package in ["--help", "good;exit", "../good", "good *", ""] {
        assert_eq!(
            check(package)["diagnostics"][0]["code"],
            "VERIFY_PACKAGE",
            "{package}"
        );
    }
}
