//! Single checked-in definition for shared schemas, availability and documentation.
use serde_json::{Value, json};
use std::sync::OnceLock;

pub fn manifest() -> &'static Value {
    static MANIFEST: OnceLock<Value> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        serde_json::from_str(include_str!("../../../contracts/tool-contract.json"))
            .expect("checked-in tool contract")
    })
}
pub fn tool_names(backend: &str) -> Vec<String> {
    manifest()["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter(|tool| {
            tool["backends"]
                .as_array()
                .expect("backends")
                .iter()
                .any(|b| b == backend)
        })
        .map(|tool| tool["name"].as_str().expect("tool name").into())
        .collect()
}
pub fn metadata(backend: &str) -> Value {
    json!({"backend":backend,"contractVersion":manifest()["contractVersion"],
        "contractHash":crate::hash(include_bytes!("../../../contracts/tool-contract.json")),
        "features":manifest()["backends"][backend],"tools":tool_names(backend)})
}

pub fn routing(backend: &str) -> Vec<Value> {
    manifest()["llmPolicy"]["routing"]
        .as_array()
        .expect("routing")
        .iter()
        .filter(|route| {
            route["backends"]
                .as_array()
                .expect("route backends")
                .iter()
                .any(|b| b == backend)
        })
        .map(|route| {
            let mut route = route.clone();
            route.as_object_mut().expect("route").remove("backends");
            route
        })
        .collect()
}

#[cfg(feature = "schema")]
macro_rules! shared_schema {
    ($ty:ty, $name:literal) => {
        impl schemars::JsonSchema for $ty {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                $name.into()
            }
            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                serde_json::from_value(manifest()["schemas"][$name].clone())
                    .expect("shared JSON schema")
            }
        }
    };
}
#[cfg(feature = "schema")]
shared_schema!(crate::contracts::ExploreArgs, "harness_explore");
#[cfg(feature = "schema")]
shared_schema!(crate::contracts::DataQueryArgs, "harness_data_query");

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routing_only_advertises_available_read_workflows() {
        for backend in ["node", "rust"] {
            let tools = tool_names(backend);
            for route in routing(backend) {
                assert!(tools.iter().any(|name| route["tool"] == *name));
                if route["tool"] == "harness_data_query" {
                    assert_eq!(backend, "rust");
                }
                if route["tool"] == "harness_explore" && backend == "rust" {
                    let mut args = route["defaults"].clone();
                    args["query"] = json!("target");
                    let args: crate::contracts::ExploreArgs = serde_json::from_value(args).unwrap();
                    assert!(!crate::contracts::Request::Explore(args).is_mutating());
                }
            }
        }
        assert!(
            manifest()["llmPolicy"]["rules"][0]
                .as_str()
                .unwrap()
                .contains("read-only")
        );
    }
    #[test]
    fn shared_contract_document_and_backends_match() {
        let mut text = format!(
            "# Shared harness contract\n\nGenerated from `contracts/tool-contract.json`. Contract version: {}.\n\n| Tool | Backends |\n| --- | --- |\n",
            manifest()["contractVersion"].as_str().unwrap()
        );
        for tool in manifest()["tools"].as_array().unwrap() {
            text.push_str(&format!(
                "| {} | {} |\n",
                tool["name"].as_str().unwrap(),
                tool["backends"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| b.as_str().unwrap())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        text.push_str("\nSchemas for explore and data_query are consumed directly by native MCP. Node integration must consume the same definitions; backend differences remain explicit.\n");
        assert_eq!(text, include_str!("../../../docs/TOOL-CONTRACT.md"));
        assert_eq!(tool_names("rust").len(), 29);
        assert_eq!(tool_names("node").len(), 31);
    }
}
