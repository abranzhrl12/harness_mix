//! Transport-independent browser inspection contracts.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One browser operation. `session` pins the browser connection to a Harness session.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserArgs {
    Connect {
        /// Local CDP discovery URL, e.g. http://localhost:9222, or a direct
        /// ws://loopback:port/devtools/browser/id URL. Chromium must already
        /// expose remote debugging; HTTP discovery permits only /json/version.
        endpoint: String,
        session: Option<String>,
    },
    Targets {
        #[serde(rename = "connectionId")]
        connection_id: String,
        session: Option<String>,
    },
    Attach {
        #[serde(rename = "connectionId")]
        connection_id: String,
        #[serde(rename = "targetId")]
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
    /// Replace a caller-provided session alias with the resolved session lease ID.
    pub fn with_session(&mut self, resolved_session: String) {
        match self {
            Self::Connect { session, .. }
            | Self::Targets { session, .. }
            | Self::Attach { session, .. }
            | Self::Inspect { session, .. }
            | Self::Errors { session, .. }
            | Self::Screenshot { session, .. }
            | Self::Disconnect { session, .. } => *session = Some(resolved_session),
        }
    }

    pub fn session(&self) -> Option<&str> {
        match self {
            Self::Connect { session, .. }
            | Self::Targets { session, .. }
            | Self::Attach { session, .. }
            | Self::Inspect { session, .. }
            | Self::Errors { session, .. }
            | Self::Screenshot { session, .. }
            | Self::Disconnect { session, .. } => session.as_deref(),
        }
    }

    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            Self::Connect { .. } | Self::Attach { .. } | Self::Disconnect { .. }
        )
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BrowserResult {
    Connected {
        #[serde(rename = "connectionId")]
        connection_id: String,
    },
    Targets {
        targets: Vec<BrowserTarget>,
    },
    Attached {
        #[serde(rename = "targetId")]
        target_id: String,
    },
    Inspected {
        snapshot: BrowserElementSnapshot,
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

#[derive(Debug, Clone, Serialize)]
pub struct BrowserTarget {
    pub id: String,
    pub title: String,
    pub url: String,
    #[serde(rename = "targetType")]
    pub target_type: String,
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
        assert!(schema["oneOf"].as_array().unwrap().iter().all(|variant| {
            variant["properties"]["action"]["type"] == "string"
        }), "{schema}");
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
            serde_json::json!({"action":"connect"}),
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
