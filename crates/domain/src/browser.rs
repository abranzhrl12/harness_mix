//! Transport-independent browser inspection contracts.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Default auto-discovery endpoint sentinel when `endpoint` is omitted on `connect` or `discover`.
pub fn default_connect_endpoint() -> String {
    "auto".to_owned()
}

/// One browser operation. `session` pins the browser connection to a Harness session.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserArgs {
    Connect {
        /// Local CDP discovery URL (e.g. `http://localhost:9222`), a direct
        /// `ws://loopback:port/devtools/browser/id` URL, `"auto"` / `"localhost"`
        /// to auto-discover an active debugging session, or a localhost tab
        /// URL/port filter (e.g. `http://localhost:3000` or `:5173`).
        /// Automatic mode launches installed Chromium with a separate profile
        /// if discovery finds no session. Set tab to the local application URL
        /// to open it with event capture attached before navigation.
        #[serde(default = "default_connect_endpoint")]
        endpoint: String,
        /// Optional tab selector (URL substring, port like `:3000`, title, or targetId)
        /// to automatically select and attach when connecting.
        #[serde(
            default,
            alias = "target",
            alias = "urlFilter",
            alias = "url_filter",
            alias = "targetId",
            alias = "target_id",
            skip_serializing_if = "Option::is_none"
        )]
        tab: Option<String>,
        session: Option<String>,
    },
    Discover {
        /// Optional CDP endpoint or `"auto"` / `"localhost"` to probe local debugging ports
        /// and `DevToolsActivePort` files.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
        /// Optional tab selector (URL substring, port, title, or targetId) to rank and
        /// highlight the best matching tab.
        #[serde(
            default,
            alias = "target",
            alias = "urlFilter",
            alias = "url_filter",
            alias = "targetId",
            alias = "target_id",
            skip_serializing_if = "Option::is_none"
        )]
        tab: Option<String>,
        session: Option<String>,
    },
    Targets {
        #[serde(rename = "connectionId")]
        connection_id: String,
        /// Optional filter (URL substring, port, title, or targetId) to narrow or highlight tabs.
        #[serde(
            default,
            alias = "target",
            alias = "urlFilter",
            alias = "url_filter",
            skip_serializing_if = "Option::is_none"
        )]
        tab: Option<String>,
        session: Option<String>,
    },
    Attach {
        #[serde(rename = "connectionId")]
        connection_id: String,
        /// Target ID, `"auto"` / `"localhost"`, or a URL/port/title filter to select and attach.
        #[serde(
            rename = "targetId",
            alias = "tab",
            alias = "target",
            alias = "urlFilter"
        )]
        target_id: String,
        session: Option<String>,
    },
    Inspect {
        #[serde(rename = "connectionId")]
        connection_id: String,
        #[serde(rename = "targetId")]
        target_id: String,
        selector: Option<String>,
        #[serde(rename = "maxChars")]
        #[cfg_attr(feature = "schema", schemars(extend("maximum" = 100000)))]
        max_chars: Option<u32>,
        session: Option<String>,
    },
    Click {
        #[serde(rename = "connectionId")]
        connection_id: String,
        #[serde(rename = "targetId")]
        target_id: String,
        selector: String,
        session: Option<String>,
    },
    Fill {
        #[serde(rename = "connectionId")]
        connection_id: String,
        #[serde(rename = "targetId")]
        target_id: String,
        selector: String,
        value: String,
        session: Option<String>,
    },
    Wait {
        #[serde(rename = "connectionId")]
        connection_id: String,
        #[serde(rename = "targetId")]
        target_id: String,
        selector: String,
        state: Option<BrowserWaitState>,
        #[serde(rename = "timeoutMs")]
        timeout_ms: Option<u32>,
        session: Option<String>,
    },
    Snapshot {
        #[serde(rename = "connectionId")]
        connection_id: String,
        #[serde(rename = "targetId")]
        target_id: String,
        limit: Option<u32>,
        session: Option<String>,
    },
    Errors {
        #[serde(rename = "connectionId")]
        connection_id: String,
        #[serde(rename = "targetId")]
        target_id: Option<String>,
        after: Option<u64>,
        limit: Option<u32>,
        session: Option<String>,
    },
    Screenshot {
        #[serde(rename = "connectionId")]
        connection_id: String,
        #[serde(rename = "targetId")]
        target_id: String,
        selector: Option<String>,
        session: Option<String>,
    },
    Disconnect {
        #[serde(rename = "connectionId")]
        connection_id: String,
        session: Option<String>,
    },
}

impl BrowserArgs {
    /// Construct a `Connect` request with an explicit endpoint and optional session.
    pub fn connect(endpoint: impl Into<String>, session: Option<String>) -> Self {
        Self::Connect {
            endpoint: endpoint.into(),
            tab: None,
            session,
        }
    }

    /// Replace a caller-provided session alias with the resolved session lease ID.
    pub fn with_session(&mut self, resolved_session: String) {
        match self {
            Self::Connect { session, .. }
            | Self::Discover { session, .. }
            | Self::Targets { session, .. }
            | Self::Attach { session, .. }
            | Self::Inspect { session, .. }
            | Self::Click { session, .. }
            | Self::Fill { session, .. }
            | Self::Wait { session, .. }
            | Self::Snapshot { session, .. }
            | Self::Errors { session, .. }
            | Self::Screenshot { session, .. }
            | Self::Disconnect { session, .. } => *session = Some(resolved_session),
        }
    }

    pub fn session(&self) -> Option<&str> {
        match self {
            Self::Connect { session, .. }
            | Self::Discover { session, .. }
            | Self::Targets { session, .. }
            | Self::Attach { session, .. }
            | Self::Inspect { session, .. }
            | Self::Click { session, .. }
            | Self::Fill { session, .. }
            | Self::Wait { session, .. }
            | Self::Snapshot { session, .. }
            | Self::Errors { session, .. }
            | Self::Screenshot { session, .. }
            | Self::Disconnect { session, .. } => session.as_deref(),
        }
    }

    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            Self::Connect { .. }
                | Self::Attach { .. }
                | Self::Disconnect { .. }
                | Self::Click { .. }
                | Self::Fill { .. }
        )
    }
}

/// Wait condition: present in the DOM, visible, or absent/hidden.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BrowserWaitState {
    Present,
    Visible,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSelectedTab {
    #[serde(rename = "targetId")]
    pub target_id: String,
    pub title: String,
    pub url: String,
}

impl From<&BrowserTarget> for BrowserSelectedTab {
    fn from(target: &BrowserTarget) -> Self {
        Self {
            target_id: target.id.clone(),
            title: target.title.clone(),
            url: target.url.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BrowserResult {
    Connected {
        #[serde(rename = "connectionId")]
        connection_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
        #[serde(rename = "selectedTab", skip_serializing_if = "Option::is_none")]
        selected_tab: Option<BrowserSelectedTab>,
        #[serde(rename = "availableTabs", skip_serializing_if = "Vec::is_empty")]
        available_tabs: Vec<BrowserTarget>,
    },
    Discovered {
        endpoint: String,
        #[serde(rename = "selectedTab", skip_serializing_if = "Option::is_none")]
        selected_tab: Option<BrowserSelectedTab>,
        #[serde(rename = "availableTabs")]
        available_tabs: Vec<BrowserTarget>,
    },
    Targets {
        targets: Vec<BrowserTarget>,
        #[serde(rename = "selectedTab", skip_serializing_if = "Option::is_none")]
        selected_tab: Option<BrowserSelectedTab>,
        #[serde(rename = "availableTabs", skip_serializing_if = "Vec::is_empty")]
        available_tabs: Vec<BrowserTarget>,
    },
    Attached {
        #[serde(rename = "targetId")]
        target_id: String,
        #[serde(rename = "selectedTab", skip_serializing_if = "Option::is_none")]
        selected_tab: Option<BrowserSelectedTab>,
        #[serde(rename = "availableTabs", skip_serializing_if = "Vec::is_empty")]
        available_tabs: Vec<BrowserTarget>,
    },
    Inspected {
        snapshot: BrowserElementSnapshot,
    },
    Clicked {},
    Filled {},
    Waited {},
    Snapshot {
        elements: Vec<BrowserInteractiveElement>,
        truncated: bool,
    },
    Errors {
        events: Vec<BrowserEventRecord>,
        #[serde(rename = "nextCursor")]
        next_cursor: u64,
        dropped: u64,
    },
    Screenshot {
        #[serde(rename = "mimeType")]
        mime_type: String,
        #[serde(rename = "dataBase64")]
        data_base64: String,
        width: u32,
        height: u32,
    },
    Disconnected {
        #[serde(rename = "connectionId")]
        connection_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserTarget {
    pub id: String,
    #[serde(rename = "targetId")]
    pub target_id: String,
    pub title: String,
    pub url: String,
    #[serde(rename = "targetType")]
    pub target_type: String,
    #[serde(rename = "isLocalhost")]
    pub is_localhost: bool,
}

impl BrowserTarget {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        url: impl Into<String>,
        target_type: impl Into<String>,
        is_localhost: bool,
    ) -> Self {
        let id = id.into();
        let target_id = id.clone();
        Self {
            id,
            target_id,
            title: title.into(),
            url: url.into(),
            target_type: target_type.into(),
            is_localhost,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserInteractiveElement {
    pub selector: String,
    pub tag: String,
    pub role: String,
    pub name: String,
    pub visible: bool,
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserElementSnapshot {
    pub tag: String,
    #[serde(rename = "outerHtml", skip_serializing_if = "Option::is_none")]
    pub outer_html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<BrowserRect>,
    #[serde(rename = "computedStyle", skip_serializing_if = "Option::is_none")]
    pub computed_style: Option<BTreeMap<String, String>>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct BrowserRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserEvent {
    Console {
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
        level: String,
        message: String,
        url: Option<String>,
        line: Option<u32>,
        column: Option<u32>,
        stack: Option<String>,
    },
    Exception {
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
        message: String,
        url: Option<String>,
        line: Option<u32>,
        column: Option<u32>,
        stack: Option<String>,
    },
    NetworkFailure {
        #[serde(rename = "timestampMs")]
        timestamp_ms: f64,
        url: String,
        error: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserEventRecord {
    pub sequence: u64,
    #[serde(rename = "targetId")]
    pub target_id: String,
    pub event: BrowserEvent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "schema")]
    #[test]
    fn schema_is_an_object_with_a_discriminated_action_union() {
        let schema = serde_json::to_value(schemars::schema_for!(BrowserArgs)).unwrap();
        assert_eq!(schema["type"], "object", "{schema}");
        assert!(schema["oneOf"].is_array(), "{schema}");
        assert!(
            schema["oneOf"]
                .as_array()
                .unwrap()
                .iter()
                .all(|variant| { variant["properties"]["action"]["type"] == "string" }),
            "{schema}"
        );
    }

    #[test]
    fn actions_deserialize_with_camel_case_and_reject_unknown_fields() {
        let args: BrowserArgs = serde_json::from_value(serde_json::json!({
            "action": "inspect",
            "connectionId": "conn-1",
            "targetId": "target-1",
            "maxChars": 4096
        }))
        .unwrap();
        assert!(matches!(
            args,
            BrowserArgs::Inspect {
                max_chars: Some(4096),
                ..
            }
        ));

        assert!(
            serde_json::from_value::<BrowserArgs>(serde_json::json!({
                "action": "inspect",
                "connectionId": "conn-1",
                "targetId": "target-1",
                "unexpected": true
            }))
            .is_err()
        );
    }

    #[test]
    fn connect_defaults_endpoint_to_auto_and_accepts_tab_aliases() {
        let args: BrowserArgs = serde_json::from_value(serde_json::json!({
            "action": "connect",
            "urlFilter": "localhost:5173"
        }))
        .unwrap();
        match args {
            BrowserArgs::Connect { endpoint, tab, .. } => {
                assert_eq!(endpoint, "auto");
                assert_eq!(tab.as_deref(), Some("localhost:5173"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn session_is_resolved_for_every_action_and_survives_round_trip() {
        let mut args: BrowserArgs = serde_json::from_value(serde_json::json!({
            "action": "attach",
            "connectionId": "conn-1",
            "targetId": "target-1",
            "session": "alias"
        }))
        .unwrap();
        assert_eq!(args.session(), Some("alias"));
        args.with_session("resolved-id".into());
        assert_eq!(args.session(), Some("resolved-id"));

        let encoded = serde_json::to_value(&args).unwrap();
        assert_eq!(encoded["action"], "attach");
        assert_eq!(encoded["session"], "resolved-id");
        assert_eq!(encoded["targetId"], "target-1");
    }

    #[test]
    fn all_actions_require_their_identifying_fields() {
        for value in [
            serde_json::json!({"action":"targets"}),
            serde_json::json!({"action":"attach","connectionId":"c"}),
            serde_json::json!({"action":"inspect","connectionId":"c"}),
            serde_json::json!({"action":"errors"}),
            serde_json::json!({"action":"screenshot","connectionId":"c"}),
            serde_json::json!({"action":"disconnect"}),
        ] {
            assert!(serde_json::from_value::<BrowserArgs>(value).is_err());
        }
    }
}

#[cfg(test)]
mod interaction_contract_tests {
    use super::BrowserArgs;
    #[cfg(feature = "schema")]
    #[test]
    fn schema_exposes_interaction_actions() {
        let schema = serde_json::to_value(schemars::schema_for!(BrowserArgs)).unwrap();
        let variants = schema["oneOf"].as_array().unwrap();
        for action in ["click", "fill", "wait", "snapshot"] {
            assert!(variants.iter().any(|v| v["properties"]["action"]["const"] == action), "missing {action}: {schema}");
        }
    }

    #[test]
    fn interaction_actions_accept_requests_and_mark_writes() {
        for (action, extra, mutating) in [
            ("click", serde_json::json!({"selector":"#save"}), true),
            (
                "fill",
                serde_json::json!({"selector":"#name", "value":"Ada"}),
                true,
            ),
            (
                "wait",
                serde_json::json!({"selector":"#save", "state":"visible", "timeoutMs":500}),
                false,
            ),
            ("snapshot", serde_json::json!({"limit":10}), false),
        ] {
            let mut value = serde_json::json!({"action": action, "connectionId":"c", "targetId":"t", "session":"s"});
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let mut args: BrowserArgs = serde_json::from_value(value).unwrap();
            assert_eq!(args.is_mutating(), mutating);
            args.with_session("resolved".into());
            assert_eq!(args.session(), Some("resolved"));
        }
    }
}
