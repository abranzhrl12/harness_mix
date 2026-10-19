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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolProfile {
    #[default]
    Full,
    Investigate,
    Edit,
}
impl ToolProfile {
    pub fn parse(name: &str) -> Result<Self, crate::Failure> {
        match name {
            "full" => Ok(Self::Full),
            "investigate" => Ok(Self::Investigate),
            "edit" => Ok(Self::Edit),
            _ => Err((
                "TOOL_PROFILE",
                "HARNESS_TOOL_PROFILE must be full, investigate or edit",
            )),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Investigate => "investigate",
            Self::Edit => "edit",
        }
    }
    pub fn allows(self, name: &str) -> bool {
        self == Self::Full
            || manifest()["toolProfiles"]["investigate"]
                .as_array()
                .expect("profile")
                .iter()
                .any(|v| v == name)
            || self == Self::Edit
                && manifest()["toolProfiles"]["edit"]
                    .as_array()
                    .expect("profile")
                    .iter()
                    .any(|v| v == name)
    }
    pub fn tools(self) -> Vec<String> {
        tool_names("rust")
            .into_iter()
            .filter(|n| self.allows(n))
            .collect()
    }
    pub fn check(self, request: &crate::contracts::Request) -> Result<(), crate::Failure> {
        use crate::contracts::Request;
        let denied = (
            "TOOL_PROFILE",
            "Tool or operation is not allowed by this server profile; request an explicitly authorized server restart with the required profile",
        );
        if !self.allows(request.name()) {
            return Err(denied);
        }
        if self != Self::Full {
            if let Request::Explore(args) = request {
                if args.sync_index == Some(true) {
                    return Err(denied);
                }
            }
            if let Request::Batch(args) = request {
                // Reject the whole batch before any child can execute.
                for job in &args.jobs {
                    let name = serde_json::to_value(&job.tool).map_err(|_| denied)?;
                    if !self.allows(name.as_str().unwrap_or("")) {
                        return Err(denied);
                    }
                }
            }
        }
        Ok(())
    }
    pub fn metadata(self) -> Value {
        let mut result = metadata("rust");
        result["tools"] = json!(self.tools());
        result["toolProfile"] = json!(self.name());
        if let Some(tools) = result["features"]["batchTools"].as_array_mut() {
            tools.retain(|name| name.as_str().is_some_and(|name| self.allows(name)));
        }
        if !self.allows("harness_jobs") {
            result["features"]
                .as_object_mut()
                .expect("features")
                .remove("backgroundTasks");
        }
        result
    }
    pub fn routing(self) -> Vec<Value> {
        routing("rust")
            .into_iter()
            .filter(|r| r["tool"].as_str().is_some_and(|n| self.allows(n)))
            .collect()
    }
    pub fn workflows(self) -> Value {
        let mut value = manifest()["llmPolicy"]["workflows"].clone();
        if self == Self::Investigate {
            value.as_object_mut().expect("workflows").remove("fix");
        }
        value
    }
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
    fn background_jobs_are_advertised_only_in_full_profile() {
        assert!(ToolProfile::Full.allows("harness_jobs"));
        assert_eq!(
            ToolProfile::Full.metadata()["features"]["backgroundTasks"]["tool"],
            "harness_jobs"
        );
        for profile in [ToolProfile::Investigate, ToolProfile::Edit] {
            assert!(!profile.allows("harness_jobs"));
            assert!(
                profile.metadata()["features"]
                    .get("backgroundTasks")
                    .is_none()
            );
            assert!(
                profile
                    .routing()
                    .iter()
                    .all(|route| route["tool"] != "harness_jobs")
            );
        }
        assert!(!tool_names("node").iter().any(|name| name == "harness_jobs"));
    }
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
        // Git may check Markdown out as CRLF on Windows; compare content, not checkout line endings.
        assert_eq!(
            text,
            include_str!("../../../docs/TOOL-CONTRACT.md").replace("\r\n", "\n")
        );
        assert_eq!(tool_names("rust").len(), 29);
        assert_eq!(tool_names("node").len(), 31);
    }
}
