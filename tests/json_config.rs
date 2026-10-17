use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{BatchArgs, ConfigArgs, DetailArgs, Request};
use serde_json::{Value, json};

fn read(app: &NativeApplication, section: &str, max: usize) -> Value {
    app.execute(Request::Config(ConfigArgs {
        path: "appsettings.json".into(),
        section: section.into(),
        max_chars: Some(max),
        external_scope: None,
        session: None,
    }))
}

#[test]
fn redacts_property_names_and_every_scalar_including_detail_and_batch() {
    let root = tempfile::tempdir().unwrap();
    let source = json!({"Settings": {"private-key": ["private-value", 12345, true, false, null, {"nested-private": "token-private"}], "$ref": "outside-private"}});
    std::fs::write(
        root.path().join("appsettings.json"),
        format!("\u{feff}{source}"),
    )
    .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = read(&app, "Settings", 8000);
    assert_eq!(result["state"], "ok", "{result}");
    let sanitized: Value = serde_json::from_str(result["data"]["text"].as_str().unwrap()).unwrap();
    assert!(sanitized.is_object());
    for secret in [
        "private-key",
        "private-value",
        "12345",
        "nested-private",
        "token-private",
        "outside-private",
        "$ref",
    ] {
        assert!(!result.to_string().contains(secret));
    }
    assert_eq!(result["data"]["externalReferencesFollowed"], false);
    assert_eq!(result["data"]["editReady"], false);
    assert_eq!(
        read(&app, "settings", 8000)["diagnostics"][0]["code"],
        "SECTION"
    );
    std::fs::write(
        root.path().join("appsettings.json"),
        json!({"Settings":vec!["detail-private";500]}).to_string(),
    )
    .unwrap();
    let result = read(&app, "Settings", 500);
    let detail = app.execute(Request::Detail(DetailArgs {
        reference: result["detail"]["ref"].as_str().unwrap().into(),
        offset: None,
        max_chars: Some(16000),
        session: None,
    }));
    assert_eq!(detail["state"], "ok");
    assert!(!detail.to_string().contains("detail-private"));
    let batch = app.execute(Request::Batch(serde_json::from_value::<BatchArgs>(json!({"jobs":[{
        "tool":"harness_config","arguments":{"path":"appsettings.json","section":"Settings","maxChars":500}
    }]})).unwrap()));
    assert_eq!(batch["state"], "ok", "{batch}");
    assert!(!batch.to_string().contains("detail-private"));
}

#[test]
fn rejects_invalid_json_and_budgets_without_parser_source() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    for source in [
        "{\"Settings\": invalid-private}".into(),
        "{\"Settings\":1} trailing-private".into(),
        format!("{{\"Settings\":{}0{}}}", "[".repeat(65), "]".repeat(65)),
        json!({"Settings":vec![0;10000]}).to_string(),
    ] {
        std::fs::write(root.path().join("appsettings.json"), source).unwrap();
        let result = read(&app, "Settings", 8000);
        assert_eq!(result["diagnostics"][0]["code"], "CONFIG_JSON", "{result}");
        assert!(!result.to_string().contains("invalid-private"));
        assert!(!result.to_string().contains("trailing-private"));
    }
    std::fs::write(
        root.path().join("appsettings.json"),
        json!({"Settings":"s".repeat(1024*1024)}).to_string(),
    )
    .unwrap();
    assert_eq!(read(&app, "Settings", 8000)["state"], "error");
    std::fs::write(root.path().join("appsettings.json"), b"\xff").unwrap();
    assert_eq!(
        read(&app, "Settings", 8000)["diagnostics"][0]["code"],
        "ENCODING"
    );
}

#[test]
fn bounded_skeleton_and_duplicate_key_policy_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    std::fs::write(
        root.path().join("appsettings.json"),
        r#"{"Settings":"first-private","Settings":{"last-private":false}}"#,
    )
    .unwrap();
    let result = read(&app, "Settings", 8000);
    assert_eq!(result["state"], "ok");
    assert!(result["data"]["text"].as_str().unwrap().contains("field_1"));
    assert!(!result.to_string().contains("first-private"));
    assert!(!result.to_string().contains("last-private"));
    let object: serde_json::Map<String, Value> = (0..5000)
        .map(|i| (format!("secret-{i}"), json!(i)))
        .collect();
    std::fs::write(
        root.path().join("appsettings.json"),
        json!({"Settings":object}).to_string(),
    )
    .unwrap();
    let result = read(&app, "Settings", 8000);
    assert_eq!(result["state"], "ok");
    assert_eq!(result["data"]["complete"], false);
    assert_eq!(result["omitted"]["truncated"], true);
    assert_eq!(result["detail"]["complete"], false);
    assert!(!result.to_string().contains("secret-"));
}
