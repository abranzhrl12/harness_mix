use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{BatchArgs, ConfigArgs, DetailArgs, Request};
use serde_json::{Value, json};
fn config(app: &NativeApplication, section: &str, max: usize) -> Value {
    app.execute(Request::Config(ConfigArgs {
        path: "Web.config".into(),
        section: section.into(),
        max_chars: Some(max),
        external_scope: None,
        session: None,
    }))
}
#[test]
fn redacts_all_values_comments_cdata_and_processing_instructions_before_truncation() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("Web.config"),r#"<configuration><connectionStrings><!-- comment-secret --><add name="db-name-secret" connectionString="Server=secret-host;Password=pwd-secret" providerName="provider-secret"><value><![CDATA[cdata-secret]]></value><?private pi-secret?></add></connectionStrings></configuration>"#).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = config(&app, "connectionStrings", 500);
    assert_eq!(result["state"], "ok", "{result}");
    let rendered = result.to_string();
    for secret in [
        "comment-secret",
        "db-name-secret",
        "secret-host",
        "pwd-secret",
        "provider-secret",
        "cdata-secret",
        "pi-secret",
    ] {
        assert!(!rendered.contains(secret), "scalar value leaked");
    }
    assert!(
        result["data"]["text"]
            .as_str()
            .unwrap()
            .contains("connectionString=\"***\"")
    );
    assert_eq!(result["data"]["externalReferencesFollowed"], false);
    assert_eq!(result["data"]["editReady"], false);
    std::fs::write(
        root.path().join("Web.config"),
        "\u{feff}<configuration><appSettings/></configuration>",
    )
    .unwrap();
    assert_eq!(config(&app, "appSettings", 8000)["state"], "ok");
    std::fs::write(
        root.path().join("Web.config"),
        format!(
            "<configuration><appSettings>{}</appSettings></configuration>",
            r#"<add key="private-key" value="private-value"/>"#.repeat(200)
        ),
    )
    .unwrap();
    let result = config(&app, "appSettings", 500);
    let detail = app.execute(Request::Detail(DetailArgs {
        reference: result["detail"]["ref"].as_str().unwrap().into(),
        offset: None,
        max_chars: Some(16000),
        session: None,
    }));
    assert!(!detail.to_string().contains("private-value"));
    assert!(!detail.to_string().contains("private-key"));
}
#[test]
fn dtd_invalid_xml_and_parser_errors_never_expose_original_content() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    for xml in [
        r#"<!DOCTYPE configuration []><configuration><appSettings/></configuration>"#,
        r#"<!DOCTYPE configuration [<!ENTITY remote SYSTEM "file:///outside-private">]><configuration><appSettings>&remote;</appSettings></configuration>"#,
        r#"<configuration><appSettings value="malformed-private"></configuration>"#,
    ] {
        std::fs::write(root.path().join("Web.config"), xml).unwrap();
        let result = config(&app, "appSettings", 8000);
        assert_eq!(result["diagnostics"][0]["code"], "CONFIG_XML", "{result}");
        assert!(!result.to_string().contains("outside-private"));
        assert!(!result.to_string().contains("malformed-private"));
    }
}
#[test]
fn rejects_ambiguous_sections_depth_node_and_input_budgets() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    std::fs::write(
        root.path().join("Web.config"),
        "<configuration><appSettings/><appSettings/></configuration>",
    )
    .unwrap();
    assert_eq!(
        config(&app, "appSettings", 8000)["diagnostics"][0]["code"],
        "AMBIGUOUS_SECTION"
    );
    assert_eq!(
        config(&app, "../invalid", 8000)["diagnostics"][0]["code"],
        "ARGUMENT"
    );
    std::fs::write(
        root.path().join("Web.config"),
        format!("<configuration>{}</configuration>", "<a/>".repeat(10001)),
    )
    .unwrap();
    assert_eq!(
        config(&app, "configuration", 8000)["diagnostics"][0]["code"],
        "CONFIG_XML"
    );
    std::fs::write(
        root.path().join("Web.config"),
        format!("{}{}", "<a>".repeat(66), "</a>".repeat(66)),
    )
    .unwrap();
    assert_eq!(
        config(&app, "a", 8000)["diagnostics"][0]["code"],
        "CONFIG_XML"
    );
    std::fs::write(root.path().join("Web.config"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
    assert_eq!(
        config(&app, "configuration", 8000)["diagnostics"][0]["code"],
        "SIZE"
    );
}
#[test]
fn batch_config_results_are_redacted_and_do_not_follow_config_source() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("Web.config"),
        r#"<configuration><appSettings configSource="../outside-private.config"/></configuration>"#,
    )
    .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let args:BatchArgs=serde_json::from_value(json!({"jobs":[{"tool":"harness_config","arguments":{"path":"Web.config","section":"appSettings"}}]})).unwrap();
    let result = app.execute(Request::Batch(args));
    assert_eq!(result["state"], "ok", "{result}");
    assert!(!result.to_string().contains("outside-private"));
}
