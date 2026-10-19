#![cfg(windows)]
use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{
    BootstrapArgs, ConfigArgs, ExternalScopeAction, ExternalScopeArgs, Kind, ReadArgs, Request,
    SearchArgs,
};
use serde_json::{Value, json};
fn session(app: &NativeApplication) -> String {
    app.execute(Request::Bootstrap(BootstrapArgs {
        compact: false,
        session: None,
        include_rules: None,
    }))["data"]["sessionId"]
        .as_str()
        .unwrap()
        .into()
}
fn grant(app: &NativeApplication, id: &str, path: &std::path::Path, authorized: bool) -> Value {
    let path = path.to_string_lossy();
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    app.execute(Request::ExternalScope(ExternalScopeArgs {
        action: ExternalScopeAction::Grant,
        session: id.into(),
        root: Some(path.into()),
        user_authorized: Some(authorized),
        scope_id: None,
    }))
}
fn read(app: &NativeApplication, id: &str, scope: &str, path: &str) -> Value {
    app.execute(Request::Read(ReadArgs {
        path: path.into(),
        start: 1,
        end: 1,
        session: Some(id.into()),
        external_scope: Some(scope.into()),
    }))
}

#[test]
fn grants_require_explicit_consent_and_remain_session_bound_and_revocable() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("a.txt"), "authorized fixture").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let first = session(&app);
    let second = session(&app);
    assert_eq!(
        grant(&app, &first, outside.path(), false)["diagnostics"][0]["code"],
        "AUTHORIZATION"
    );
    assert_eq!(
        grant(&app, &first, std::path::Path::new("relative"), true)["diagnostics"][0]["code"],
        "PATH"
    );
    let granted = grant(&app, &first, outside.path(), true);
    assert_eq!(granted["state"], "ok", "{granted}");
    let scope = granted["data"]["scopeId"].as_str().unwrap();
    assert_eq!(
        read(&app, &first, scope, "a.txt")["data"]["text"],
        "authorized fixture"
    );
    assert_eq!(
        read(&app, &second, scope, "a.txt")["diagnostics"][0]["code"],
        "EXTERNAL_SCOPE"
    );
    assert_eq!(
        read(&app, &first, scope, "../adjacent.txt")["diagnostics"][0]["code"],
        "PATH"
    );
    let revoked = app.execute(Request::ExternalScope(ExternalScopeArgs {
        action: ExternalScopeAction::Revoke,
        session: first.clone(),
        root: None,
        user_authorized: None,
        scope_id: Some(scope.into()),
    }));
    assert_eq!(revoked["data"]["revoked"], true);
    assert_eq!(
        read(&app, &first, scope, "a.txt")["diagnostics"][0]["code"],
        "EXTERNAL_SCOPE"
    );
}
#[test]
fn single_file_scope_never_authorizes_its_neighbors_and_defaults_search_to_that_file() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("a.rs"), "pub fn allowed() {}\n").unwrap();
    std::fs::write(outside.path().join("a_neighbor.rs"), "private neighbor").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let id = session(&app);
    let granted = grant(&app, &id, &outside.path().join("a.rs"), true);
    assert_eq!(granted["state"], "ok", "{granted}");
    let scope = granted["data"]["scopeId"].as_str().unwrap();
    assert_eq!(read(&app, &id, scope, "a.rs")["state"], "ok");
    assert_eq!(
        read(&app, &id, scope, "a_neighbor.rs")["diagnostics"][0]["code"],
        "PATH"
    );
    let search = app.execute(Request::Search(SearchArgs {
        kind: Kind::Files,
        query: "a".into(),
        min_lines: None,
        scope: None,
        limit: None,
        session: Some(id.clone()),
        external_scope: Some(scope.into()),
    }));
    assert_eq!(search["state"], "ok", "{search}");
    assert_eq!(search["data"]["paths"], json!(["a.rs"]));
    assert!(!search.to_string().contains("private neighbor"));
    let syntax = app.execute(Request::Search(SearchArgs {
        kind: Kind::Syntax,
        query: "function_item:allowed".into(),
        min_lines: None,
        scope: None,
        limit: None,
        session: Some(id),
        external_scope: Some(scope.into()),
    }));
    assert_eq!(syntax["state"], "ok", "{syntax}");
    assert_eq!(syntax["data"]["externalScope"], scope);
    assert_eq!(syntax["data"]["matches"].as_array().unwrap().len(), 1);
    assert_eq!(syntax["data"]["matches"][0]["file"], "a.rs");
    assert!(!syntax.to_string().contains("private neighbor"));
}
#[test]
fn external_config_requires_redaction_and_grants_do_not_survive_restart() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("Web.config"),r#"<configuration><connectionStrings><add connectionString="Password=external-private"/></connectionStrings></configuration>"#).unwrap();
    let app = adapters::application_with_logging(root.path(), true).unwrap();
    let id = session(&app);
    let granted = grant(&app, &id, &outside.path().join("Web.config"), true);
    assert_eq!(granted["logPersistence"]["saved"], true, "{granted}");
    let scope = granted["data"]["scopeId"].as_str().unwrap().to_owned();
    assert_eq!(
        read(&app, &id, &scope, "Web.config")["diagnostics"][0]["code"],
        "PATH"
    );
    let redacted = app.execute(Request::Config(ConfigArgs {
        path: "Web.config".into(),
        section: "connectionStrings".into(),
        max_chars: None,
        external_scope: Some(scope.clone()),
        session: Some(id.clone()),
        raw: None,
    }));
    assert_eq!(redacted["state"], "ok", "{redacted}");
    assert_eq!(redacted["logPersistence"]["saved"], true);
    assert!(!redacted.to_string().contains("external-private"));
    std::fs::write(
        outside.path().join("appsettings.json"),
        r#"{"Settings":{"password-private":"json-private"}}"#,
    )
    .unwrap();
    let json_scope = grant(&app, &id, &outside.path().join("appsettings.json"), true);
    let json_scope = json_scope["data"]["scopeId"].as_str().unwrap();
    assert_eq!(
        read(&app, &id, json_scope, "appsettings.json")["diagnostics"][0]["code"],
        "PATH"
    );
    let redacted = app.execute(Request::Config(ConfigArgs {
        path: "appsettings.json".into(),
        section: "Settings".into(),
        max_chars: None,
        external_scope: Some(json_scope.into()),
        session: Some(id.clone()),
        raw: None,
    }));
    assert_eq!(redacted["state"], "ok", "{redacted}");
    assert_eq!(redacted["logPersistence"]["saved"], true);
    assert_eq!(redacted["data"]["externalScope"], json_scope);
    assert!(!redacted.to_string().contains("password-private"));
    assert!(!redacted.to_string().contains("json-private"));
    std::fs::write(outside.path().join("appsettings.json"), "{invalid-private}").unwrap();
    let invalid = app.execute(Request::Config(ConfigArgs {
        path: "appsettings.json".into(),
        section: "Settings".into(),
        max_chars: None,
        external_scope: Some(json_scope.into()),
        session: Some(id.clone()),
        raw: None,
    }));
    assert_eq!(invalid["diagnostics"][0]["code"], "CONFIG_JSON");
    assert_eq!(invalid["logPersistence"]["saved"], true);
    assert!(!invalid.to_string().contains("invalid-private"));
    drop(app);
    let restarted = adapters::application(root.path()).unwrap();
    assert_eq!(
        read(&restarted, &id, &scope, "Web.config")["diagnostics"][0]["code"],
        "EXTERNAL_SCOPE"
    );
}
#[test]
fn replacing_the_authorized_directory_invalidates_its_identity() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let allowed = outside.path().join("allowed");
    std::fs::create_dir(&allowed).unwrap();
    std::fs::write(allowed.join("a.txt"), "before").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let id = session(&app);
    let granted = grant(&app, &id, &allowed, true);
    let scope = granted["data"]["scopeId"].as_str().unwrap();
    std::fs::rename(&allowed, outside.path().join("old-target")).unwrap();
    std::fs::create_dir(&allowed).unwrap();
    std::fs::write(allowed.join("a.txt"), "replacement-private").unwrap();
    let stale = read(&app, &id, scope, "a.txt");
    assert_eq!(stale["diagnostics"][0]["code"], "STALE_SCOPE", "{stale}");
    assert!(!stale.to_string().contains("replacement-private"));
}
#[test]
fn hardlinks_sensitive_backups_and_volume_roots_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("original.txt"), "hardlink private").unwrap();
    std::fs::hard_link(
        outside.path().join("original.txt"),
        outside.path().join("alias.txt"),
    )
    .unwrap();
    std::fs::write(
        outside.path().join("mcp_config.json.backup"),
        "private backup",
    )
    .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let id = session(&app);
    for path in [
        outside.path().join("alias.txt"),
        outside.path().join("mcp_config.json.backup"),
        std::path::PathBuf::from("C:\\"),
    ] {
        assert_eq!(
            grant(&app, &id, &path, true)["diagnostics"][0]["code"],
            "PATH"
        );
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        assert_eq!(
            grant(&app, &id, std::path::Path::new(&profile), true)["diagnostics"][0]["code"],
            "PATH"
        );
    }
    let granted = grant(&app, &id, outside.path(), true);
    let scope = granted["data"]["scopeId"].as_str().unwrap();
    assert_eq!(
        read(&app, &id, scope, "alias.txt")["diagnostics"][0]["code"],
        "PATH"
    );
}
