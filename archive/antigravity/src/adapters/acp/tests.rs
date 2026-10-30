use super::*;

#[test]
fn image_payload_contains_bytes_instead_of_a_file_path() {
    let image = ImageAttachment { mime_type: "image/png".into(), data: vec![0, 1, 2, 255] };
    let prompt = encode_prompt("Describe esta imagen", &[image]).unwrap();
    assert_eq!(prompt[0]["type"], "text");
    assert_eq!(prompt[1]["type"], "image");
    assert_eq!(prompt[1]["mimeType"], "image/png");
    assert_eq!(base64::engine::general_purpose::STANDARD.decode(prompt[1]["data"].as_str().unwrap()).unwrap(), vec![0, 1, 2, 255]);
    assert!(encode_prompt("", &[]).is_err());
    assert!(encode_prompt("x", &[ImageAttachment {mime_type:"image/svg+xml".into(),data:b"<svg/>".to_vec()}]).is_err());
}

#[test]
fn only_advertised_auth_methods_are_exposed_and_terminal_is_identified() {
    let methods = parse_auth_methods(Some(&json!([
        {"id":"oauth-personal","name":"Log in with Google","description":"Use Google","type":"agent"},
        {"id":"terminal","name":"Terminal login","type":"terminal"}
    ])));

    assert_eq!(methods.len(), 2);
    assert_eq!(methods[0].method.id, "oauth-personal");
    assert_eq!(methods[0].method.name, "Log in with Google");
    assert_eq!(methods[1].kind, "terminal");
    assert!(
        methods[1]
            .method
            .description
            .as_deref()
            .unwrap()
            .contains("interactive terminal")
    );
    assert!(parse_auth_methods(Some(&json!(null))).is_empty());
}

#[test]
fn permission_outcomes_match_acp_and_reject_unadvertised_choices() {
    let options = vec![PermissionOption {
        id: "allow-once".into(),
        name: "Allow once".into(),
        kind: "allow_once".into(),
    }];

    let selected = permission_outcome(PermissionDecision::Selected("allow-once".into()), &options);
    assert_eq!(
        selected.pointer("/outcome/outcome").and_then(Value::as_str),
        Some("selected")
    );
    assert_eq!(
        selected
            .pointer("/outcome/optionId")
            .and_then(Value::as_str),
        Some("allow-once")
    );

    let invalid = permission_outcome(
        PermissionDecision::Selected("allow-always".into()),
        &options,
    );
    assert_eq!(
        invalid.pointer("/outcome/outcome").and_then(Value::as_str),
        Some("cancelled")
    );
    let cancelled = permission_outcome(PermissionDecision::Cancelled, &options);
    assert_eq!(
        cancelled
            .pointer("/outcome/outcome")
            .and_then(Value::as_str),
        Some("cancelled")
    );
}

#[test]
fn protocol_line_parser_requires_newline_json_and_enforces_limit() {
    assert_eq!(
        parse_protocol_line(br#"{"jsonrpc":"2.0","id":1,"result":{}}"#),
        Err(ConversationError::Other(
            "ACP message was not newline-delimited".into()
        ))
    );
    assert!(parse_protocol_line(b"{not json}\n").is_err());
    let mut oversized = vec![b' '; MAX_PROTOCOL_LINE_BYTES + 1];
    oversized.push(b'\n');
    assert_eq!(
        parse_protocol_line(&oversized),
        Err(ConversationError::Other(
            "ACP message exceeded the 8 MiB limit".into()
        ))
    );
    assert_eq!(
        parse_protocol_line(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n").unwrap()["id"],
        1
    );
}

#[test]
fn auth_required_error_maps_by_acp_code_and_other_errors_stay_other() {
    assert_eq!(
        classify_rpc_error(&json!({"code":-32000,"message":"Authentication required"})),
        Some(ConversationError::AuthenticationRequired(
            "Authentication required".into()
        ))
    );
    assert_eq!(
        classify_rpc_error(&json!({"code":-32602,"message":"invalid params"})),
        None
    );
}

#[test]
fn only_google_login_urls_are_forwarded_from_stderr() {
    let url = "https://accounts.google.com/o/oauth2/auth?state=one&client=two";
    assert_eq!(
        google_auth_url(&format!("Open this link: {url}")),
        Some(url.to_owned())
    );
    assert_eq!(
        google_auth_url("diagnostic https://example.com/not-login"),
        None
    );
    assert_eq!(
        google_auth_url(&format!(
            "https://accounts.google.com/{}",
            "x".repeat(MAX_AUTH_URL_BYTES)
        )),
        None
    );
}

#[test]
fn agent_text_and_thought_updates_map_to_distinct_events() {
    let observer = RecordingObserver::default();
    handle_update(None, &json!({"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"answer"}}}), &observer).unwrap();
    handle_update(None, &json!({"update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"private thought"}}}), &observer).unwrap();
    assert_eq!(
        *observer.events.lock().unwrap(),
        vec![
            ConversationEvent::Text("answer".into()),
            ConversationEvent::Thought("private thought".into()),
        ]
    );
}

#[test]
fn session_config_options_parse_flat_and_grouped_values_and_validate_selections() {
    let options = parse_config_options(Some(&json!([
        {
            "id": "model", "name": "Model", "category": "model",
            "type": "select", "currentValue": "g1",
            "options": [
                {"value":"g1", "name":"Gemini"},
                {"group":"recommended", "name":"Recommended", "options":[
                    {"value":"g2", "name":"Gemini Pro"}
                ]}
            ]
        },
        {
            "id": "effort", "name": "Effort", "type": "select", "currentValue": "medium",
            "options": [{"value":"low", "name":"Low"}, {"value":"medium", "name":"Medium"}]
        },
        {
            "id": "ignored-boolean", "name": "Flag", "type": "boolean", "currentValue": true
        },
        {"id":"incomplete", "name":"Incomplete", "type":"select", "options":[]}
    ])));

    assert_eq!(options.len(), 2);
    assert_eq!(options[0].id, "model");
    assert_eq!(options[0].category.as_deref(), Some("model"));
    assert_eq!(options[0].current_value, "g1");
    assert_eq!(options[0].options.iter().map(|value| value.value.as_str()).collect::<Vec<_>>(), vec!["g1", "g2"]);
    assert_eq!(options[1].category, None);
    assert!(advertises_config_value(&options, "model", "g2"));
    assert!(!advertises_config_value(&options, "model", "not-advertised"));
    assert!(!advertises_config_value(&options, "unknown", "g1"));
    assert!(parse_config_options(None).is_empty());
}

#[test]
fn session_usage_is_emitted_only_with_reported_window_values() {
    let usage = parse_session_usage(&json!({
        "used": 53000,
        "size": 200000,
        "cost": {"amount":0.045,"currency":"USD"}
    })).unwrap();
    assert_eq!(usage.used, 53000);
    assert_eq!(usage.size, 200000);
    assert_eq!(usage.cost, Some(UsageCost { amount: 0.045, currency: "USD".into() }));

    let without_cost = parse_session_usage(&json!({"used":12,"size":100,"cost":null})).unwrap();
    assert_eq!(without_cost.cost, None);
    assert!(parse_session_usage(&json!({"used":12})).is_none());
    assert!(parse_session_usage(&json!({"used":12,"size":0})).is_none());
}

#[test]
fn config_and_usage_updates_become_session_events() {
    let observer = RecordingObserver::default();
    let config_options = json!([{
        "id":"effort", "name":"Effort", "type":"select", "currentValue":"medium",
        "options":[{"value":"low","name":"Low"},{"value":"medium","name":"Medium"}]
    }]);
    handle_update(None, &json!({"sessionId":"s1","update":{"sessionUpdate":"config_option_update","configOptions":config_options}}), &observer).unwrap();
    handle_update(None, &json!({"sessionId":"s1","update":{"sessionUpdate":"usage_update","used":50,"size":100}}), &observer).unwrap();
    handle_update(None, &json!({"sessionId":"s1","update":{"sessionUpdate":"usage_update","used":50}}), &observer).unwrap();

    let events = observer.events.lock().unwrap();
    assert!(matches!(&events[0], ConversationEvent::ConfigOptions(options) if options.len() == 1 && options[0].id == "effort"));
    assert!(matches!(&events[1], ConversationEvent::Usage(usage) if usage.used == 50 && usage.size == 100 && usage.cost.is_none()));
    assert_eq!(events.len(), 2);
}

#[cfg(windows)]
#[test]
fn protocol_path_normalizes_verbatim_drive_and_unc_paths_without_loss() {
    assert_eq!(
        protocol_path(Path::new(r"\\?\E:\harness_mix\código\猫")),
        PathBuf::from(r"E:\harness_mix\código\猫")
    );
    assert_eq!(
        protocol_path(Path::new(r"\\?\UNC\server\share\proyecto ñ")),
        PathBuf::from(r"\\server\share\proyecto ñ")
    );
    let ordinary = PathBuf::from(r"E:\harness_mix\código");
    assert_eq!(protocol_path(&ordinary), ordinary);
    let ordinary_unc = PathBuf::from(r"\\server\share\proyecto ñ");
    assert_eq!(protocol_path(&ordinary_unc), ordinary_unc);
}

#[derive(Default)]
struct RecordingObserver {
    events: std::sync::Mutex<Vec<ConversationEvent>>,
}
impl ConversationObserver for RecordingObserver {
    fn on_event(&self, event: ConversationEvent) -> Result<(), String> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
    fn request_permission(
        &self,
        _request: PermissionRequest,
    ) -> Result<PermissionDecision, String> {
        Ok(PermissionDecision::Cancelled)
    }
}
