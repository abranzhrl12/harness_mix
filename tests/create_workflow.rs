#![cfg(windows)]
use agent_harness::adapters;
use harness_domain::contracts::{BootstrapArgs, Request, WriteArgs};
use serde_json::{Value, json};

fn write(app: &adapters::NativeApplication, value: Value) -> Value {
    app.execute(Request::Write(
        serde_json::from_value::<WriteArgs>(value).unwrap(),
    ))
}

#[test]
fn creates_parents_and_returns_confirmation_without_source_echo() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let request = json!({"path":"src/entities/profile.rs","content":"pub struct Profile;\n","createParents":true});
    let result = write(&app, request.clone());
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["status"], "created");
    assert_eq!(
        result["data"]["createdDirectories"],
        json!(["src", "src/entities"])
    );
    assert_eq!(result["data"]["verified"], true);
    assert_eq!(result["data"]["typecheck"], "not_run");
    assert_eq!(result["data"]["changeSummary"]["addedLines"], 1);
    assert!(result["data"].get("content").is_none());
    let retry = write(&app, request);
    assert_eq!(retry["state"], "ok", "{retry}");
    assert_eq!(retry["data"]["status"], "already_present");
    assert_eq!(retry["data"]["changed"], false);
    assert_eq!(retry["data"]["createdDirectories"], json!([]));
    assert_eq!(retry["data"]["hash"], result["data"]["hash"]);
    assert_eq!(
        std::fs::read_to_string(root.path().join("src/entities/profile.rs")).unwrap(),
        "pub struct Profile;\n"
    );
}

#[test]
fn conflicts_and_byte_differences_never_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    std::fs::write(root.path().join("existing.txt"), "a\r\n").unwrap();
    for content in ["different", "a\n"] {
        let result = write(&app, json!({"path":"existing.txt","content":content}));
        assert_eq!(result["state"], "error", "{result}");
        assert_eq!(result["diagnostics"][0]["code"], "WRITE_EXISTS");
        assert_eq!(result["data"]["changed"], false);
        assert_eq!(
            std::fs::read(root.path().join("existing.txt")).unwrap(),
            b"a\r\n"
        );
    }
}

#[test]
fn validation_precedes_directory_creation_and_reports_json_position() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    for (path, content) in [("new/deep/a.rs", "fn {"), ("new/deep/a.json", "{\"a\": }")] {
        let result = write(
            &app,
            json!({"path":path,"content":content,"createParents":true,"allowUnvalidatedText":true}),
        );
        assert_eq!(result["state"], "error", "{result}");
        assert_eq!(result["diagnostics"][0]["code"], "SYNTAX_ERROR");
        assert_eq!(result["data"]["createdDirectories"], json!([]));
        if path.ends_with("json") {
            assert_eq!(result["diagnostics"][0]["line"], 1);
            assert!(result["diagnostics"][0]["column"].as_u64().unwrap() > 0);
        }
        assert!(!root.path().join("new").exists());
    }
}

#[test]
fn text_mode_is_explicit_and_cannot_bypass_paths_or_parser_limits() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let denied = write(
        &app,
        json!({"path":"config/service.toml","content":"[service]\n","createParents":true}),
    );
    assert_eq!(denied["diagnostics"][0]["code"], "PARSER_UNAVAILABLE");
    assert!(!root.path().join("config").exists());
    let allowed = write(
        &app,
        json!({"path":"config/service.toml","content":"[service]\n","createParents":true,"allowUnvalidatedText":true}),
    );
    assert_eq!(allowed["state"], "ok", "{allowed}");
    assert_eq!(allowed["data"]["syntax"]["status"], "not_checked");
    for path in [
        "../outside/a.txt",
        ".agents/a.txt",
        ".codegraph/a.txt",
        ".git/a.txt",
        "AGENTS.md",
        ".env",
        "deep/.env/a.txt",
    ] {
        assert_eq!(
            write(
                &app,
                json!({"path":path,"content":"text","createParents":true,"allowUnvalidatedText":true})
            )["state"],
            "error",
            "{path}"
        );
    }
    assert!(!root.path().join("deep").exists());
    // An explicit opt-in tolerates parser budget limits: the write proceeds
    // with syntax marked not_checked instead of blocking the caller.
    let oversized = write(
        &app,
        json!({"path":"too/deep/a.rs","content":" ".repeat(256 * 1024 + 1),"createParents":true,"allowUnvalidatedText":true}),
    );
    assert_eq!(oversized["state"], "ok", "{oversized}");
    assert_eq!(oversized["data"]["syntax"]["status"], "not_checked", "{oversized}");
    assert_eq!(
        oversized["data"]["syntax"]["reason"],
        "parser_limit",
        "{oversized}"
    );
    assert!(root.path().join("too/deep/a.rs").exists());
    // Without the opt-in the parser budget still rejects the write outright.
    let denied = write(
        &app,
        json!({"path":"big/a.rs","content":" ".repeat(256 * 1024 + 1),"createParents":true}),
    );
    assert_eq!(denied["diagnostics"][0]["code"], "PARSER_LIMIT", "{denied}");
    assert!(!root.path().join("big").exists());
}

#[test]
fn missing_parent_opt_in_and_depth_budget_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = write(&app, json!({"path":"missing/a.txt","content":"x"}));
    assert_eq!(result["state"], "error");
    assert!(!root.path().join("missing").exists());
    let path = format!("{}a.txt", "level/".repeat(33));
    let result = write(
        &app,
        json!({"path":path,"content":"x","createParents":true}),
    );
    assert_eq!(result["state"], "error");
    assert!(!root.path().join("level").exists());
}

#[test]
fn junction_parents_are_rejected_without_external_writes() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let junction = root.path().join("link");
    let output = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let result = write(
        &app,
        json!({"path":"link/new/a.txt","content":"x","createParents":true}),
    );
    assert_eq!(result["state"], "error");
    assert!(!outside.path().join("new").exists());
    std::fs::remove_dir(junction).unwrap();
}

#[test]
fn compact_initial_bootstrap_ignores_legacy_rules_and_keeps_safety_context() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join(".agents/rules")).unwrap();
    std::fs::write(
        root.path().join(".agents/rules/00-mandatory-tools.md"),
        "Project-specific instruction: retain this rule.\n",
    )
    .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let call = |compact| {
        app.execute(Request::Bootstrap(
            serde_json::from_value::<BootstrapArgs>(json!({"compact":compact})).unwrap(),
        ))
    };
    let full = call(false);
    let compact = call(true);
    assert_eq!(compact["state"], "ok", "{compact}");
    assert_eq!(compact["data"]["catalogCompact"], true);
    assert!(compact["data"].get("rules").is_none());
    assert!(full["data"].get("rules").is_none());
    assert!(!compact.to_string().contains("retain this rule"));
    assert_eq!(compact["data"]["limitations"], full["data"]["limitations"]);
    assert!(
        compact["data"].get("executionPolicy").is_none()
            && full["data"].get("executionPolicy").is_none()
    );
    assert_eq!(compact["data"]["usageRules"], full["data"]["usageRules"]);
    assert!(compact["detail"]["ref"].is_string());
    assert!(compact["data"].get("supportedTools").is_none());
    assert!(compact.to_string().len() < full.to_string().len());
}

#[test]
fn replace_returns_summary_and_confirmation_in_one_response() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    std::fs::write(root.path().join("file.txt"), "old\n").unwrap();
    let result = app.execute(Request::Replace(
        serde_json::from_value(json!({"path":"file.txt","oldText":"old","newText":"new\nline"}))
            .unwrap(),
    ));
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["status"], "replaced");
    assert_eq!(result["data"]["changeSummary"]["lineDelta"], 1);
    assert_eq!(result["data"]["verified"], true);
    assert_eq!(result["data"]["typecheck"], "not_run");
}

#[test]
fn expected_hash_turns_create_into_atomic_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let created = write(&app, json!({"path":"note.txt","content":"first\n"}));
    let base = created["data"]["hash"].as_str().unwrap().to_owned();
    let replaced = write(
        &app,
        json!({"path":"note.txt","content":"second\n","expectedHash":base}),
    );
    assert_eq!(replaced["state"], "ok", "{replaced}");
    assert_eq!(replaced["data"]["status"], "replaced");
    assert_eq!(replaced["data"]["created"], false);
    assert_eq!(replaced["data"]["previousHash"], base);
    assert_eq!(
        std::fs::read_to_string(root.path().join("note.txt")).unwrap(),
        "second\n"
    );
    let current = replaced["data"]["hash"].as_str().unwrap().to_owned();
    let unchanged = write(
        &app,
        json!({"path":"note.txt","content":"second\n","expectedHash":current}),
    );
    assert_eq!(unchanged["state"], "ok", "{unchanged}");
    assert_eq!(unchanged["data"]["status"], "unchanged");
    assert_eq!(unchanged["data"]["changed"], false);
    let stale = write(
        &app,
        json!({"path":"note.txt","content":"third\n","expectedHash":base}),
    );
    assert_eq!(stale["diagnostics"][0]["code"], "STALE");
    assert_eq!(
        std::fs::read_to_string(root.path().join("note.txt")).unwrap(),
        "second\n"
    );
    let missing = write(
        &app,
        json!({"path":"absent.txt","content":"x","expectedHash":base}),
    );
    assert_eq!(missing["diagnostics"][0]["code"], "PATH");
    assert!(!root.path().join("absent.txt").exists());
}

#[test]
fn compact_bootstrap_keeps_catalog_without_legacy_rules_in_detail() {
    use harness_domain::contracts::{DataQueryArgs, Request};
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join(".agents/rules")).unwrap();
    let rules = "retain-rule\n".repeat(1300);
    std::fs::write(
        root.path().join(".agents/rules/00-mandatory-tools.md"),
        &rules,
    )
    .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let boot = app.execute(Request::Bootstrap(
        serde_json::from_value(json!({"compact":true})).unwrap(),
    ));
    assert_eq!(boot["state"], "ok", "{boot}");
    let query: DataQueryArgs = serde_json::from_value(json!({"resultRef":boot["detail"]["ref"],
        "query":"{rules: rules, exploreRoute: catalog.routing[?tool=='harness_explore']}","maxChars":32000}))
    .unwrap();
    let result = app.execute(Request::DataQuery(query));
    assert_eq!(result["state"], "ok", "{result}");
    let text = result.to_string();
    assert!(text.contains("harness_explore"));
    assert!(!text.contains("retain-rule"));
    assert!(boot["omitted"].get("truncated").is_none());
}
