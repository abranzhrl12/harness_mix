//! Apply tool visibility and schema narrowing at the protocol boundary.
use super::McpHarness;
use harness_application::ports::{SessionRepository, Workspace};
use harness_domain::tool_contract::ToolProfile;
use rmcp::handler::server::router::tool::ToolRouter;

fn restrict_batch_schema(
    value: &mut serde_json::Value,
    profile: harness_domain::tool_contract::ToolProfile,
) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::Array(choices)) = map.get_mut("enum") {
                if !choices.is_empty()
                    && choices
                        .iter()
                        .all(|v| v.as_str().is_some_and(|s| s.starts_with("harness_")))
                {
                    choices.retain(|v| v.as_str().is_some_and(|name| profile.allows(name)));
                }
            }
            for value in map.values_mut() {
                restrict_batch_schema(value, profile);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                restrict_batch_schema(value, profile);
            }
        }
        _ => {}
    }
}

pub(super) fn router<W, S>(
    mut router: ToolRouter<McpHarness<W, S>>,
    profile: ToolProfile,
) -> ToolRouter<McpHarness<W, S>>
where
    W: Workspace,
    S: SessionRepository,
{
    router.map.retain(|name, _| profile.allows(name));
    if profile != harness_domain::tool_contract::ToolProfile::Full {
        if let Some(route) = router.map.get_mut("harness_batch") {
            let mut schema = serde_json::Value::Object((*route.attr.input_schema).clone());
            restrict_batch_schema(&mut schema, profile);
            route.attr.input_schema =
                std::sync::Arc::new(schema.as_object().expect("tool schema").clone());
        }
    }
    let eager =
        std::env::var_os("HARNESS_MCP_EAGER_TOOLS").as_deref() != Some(std::ffi::OsStr::new("0"));
    if eager {
        for (name, route) in &mut router.map {
            if harness_domain::tool_contract::manifest()["alwaysLoadTools"]
                .as_array()
                .expect("eager tools")
                .iter()
                .any(|v| v == name.as_ref())
            {
                let mut meta = route
                    .attr
                    .meta
                    .take()
                    .map(|m| serde_json::to_value(m).expect("tool metadata"))
                    .unwrap_or_else(|| serde_json::json!({}));
                meta["antigravity/alwaysLoad"] = serde_json::json!(true);
                route.attr.meta = Some(serde_json::from_value(meta).expect("tool metadata"));
            }
        }
    }
    router
}
