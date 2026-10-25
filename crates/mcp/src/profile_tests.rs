use super::*;

#[test]
fn each_profile_exposes_only_registered_rust_tools() {
    let registered = harness_domain::tool_contract::tool_names("rust");

    for profile in [ToolProfile::Full, ToolProfile::Investigate, ToolProfile::Edit] {
        let exposed = profile.tools();
        let expected = registered
            .iter()
            .filter(|name| profile.allows(name))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(exposed, expected);
    }

    assert_eq!(ToolProfile::Full.tools(), registered);
}

#[test]
fn batch_schema_tool_choices_follow_the_selected_profile() {
    let mut schema = serde_json::json!({
        "properties": {
            "tool": { "enum": harness_domain::tool_contract::tool_names("rust") },
            "mode": { "enum": ["safe", "fast"] }
        }
    });

    restrict_batch_schema(&mut schema, ToolProfile::Investigate);

    let choices = schema["properties"]["tool"]["enum"].as_array().unwrap();
    assert!(!choices.is_empty());
    assert!(choices.iter().all(|name| {
        name.as_str()
            .is_some_and(|name| ToolProfile::Investigate.allows(name))
    }));
    assert_eq!(
        schema["properties"]["mode"]["enum"],
        serde_json::json!(["safe", "fast"])
    );
}
