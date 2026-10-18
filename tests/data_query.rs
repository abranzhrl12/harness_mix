use agent_harness::adapters;
use harness_domain::contracts::{DataQueryArgs, Request};
use serde_json::json;

#[test]
fn test_data_query_direct_data_filter_and_projection() {
    let dir = tempfile::tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();

    let data = json!({
        "services": [
            {"name": "auth", "port": 8080, "active": true},
            {"name": "billing", "port": 8081, "active": false},
            {"name": "gateway", "port": 80, "active": true}
        ]
    });

    let res = app.execute(Request::DataQuery(DataQueryArgs {
        session: None,
        path: None,
        data: Some(data),
        query: "services[?active == `true`].name".to_string(),
        result_ref: None,
        max_chars: Some(1000),
    }));

    assert_eq!(res["state"], "ok", "{res}");
    let data_res = &res["data"];
    assert_eq!(res["omitted"]["truncated"], serde_json::Value::Null);
    assert_eq!(data_res["result"], json!(["auth", "gateway"]));
}

#[test]
fn test_data_query_from_json_file() {
    let dir = tempfile::tempdir().unwrap();
    let json_content = r#"{
        "dependencies": {
            "serde": "1.0",
            "jmespath": "0.3",
            "tokio": "1.40"
        },
        "meta": {
            "maintainers": ["Alice", "Bob"]
        }
    }"#;
    std::fs::write(dir.path().join("package.json"), json_content).unwrap();

    let app = adapters::application(dir.path()).unwrap();

    let res = app.execute(Request::DataQuery(DataQueryArgs {
        session: None,
        path: Some("package.json".to_string()),
        data: None,
        query: "meta.maintainers[0]".to_string(),
        result_ref: None,
        max_chars: None,
    }));

    assert_eq!(res["state"], "ok", "{res}");
    let data_res = &res["data"];
    assert_eq!(res["omitted"]["truncated"], serde_json::Value::Null);
    assert_eq!(data_res["result"], json!("Alice"));
}

#[test]
fn test_data_query_invalid_syntax_error() {
    let dir = tempfile::tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();

    let res = app.execute(Request::DataQuery(DataQueryArgs {
        session: None,
        path: None,
        data: Some(json!({"a": 1})),
        query: "unclosed[?bracket".to_string(),
        result_ref: None,
        max_chars: None,
    }));

    assert_eq!(res["state"], "error");
    assert_eq!(res["diagnostics"][0]["code"], "QUERY_SYNTAX");
}

#[test]
fn test_data_query_truncation_when_exceeding_max_chars() {
    let dir = tempfile::tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();

    let large_array: Vec<String> = (0..500).map(|i| format!("item-{}", i)).collect();
    let data = json!({ "items": large_array });

    let res = app.execute(Request::DataQuery(DataQueryArgs {
        session: None,
        path: None,
        data: Some(data),
        query: "items".to_string(),
        result_ref: None,
        max_chars: Some(500),
    }));

    assert_eq!(res["state"], "ok", "{res}");
    assert_eq!(res["omitted"]["truncated"], true);
    assert!(res["data"]["preview"].as_str().unwrap().len() <= 500);
}
