//! Transport-independent concepts for an interactive coding-agent conversation.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub agent_name: String,
    pub auth_methods: Vec<AuthMethod>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: String,
    pub config_options: Vec<SessionConfigOption>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionConfigOption {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub current_value: String,
    pub options: Vec<ConfigValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigValue {
    pub value: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UsageCost {
    pub amount: f64,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionUsage {
    pub used: u64,
    pub size: u64,
    pub cost: Option<UsageCost>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageAttachment {
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConversationEvent {
    Text(String),
    Thought(String),
    ToolCall { title: String, status: String },
    Status(String),
    LoginUrl(String),
    ConfigOptions(Vec<SessionConfigOption>),
    Usage(SessionUsage),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRequest {
    pub title: String,
    pub options: Vec<PermissionOption>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PermissionPolicy {
    #[default]
    Ask,
    AutoApprove,
}

impl PermissionPolicy {
    /// Resolves requests only when the selected policy can do so safely.
    pub fn decision(&self, request: &PermissionRequest) -> Option<PermissionDecision> {
        match self {
            Self::Ask => None,
            Self::AutoApprove => request
                .options
                .iter()
                .find(|option| option.kind == "allow_once")
                .or_else(|| {
                    request
                        .options
                        .iter()
                        .find(|option| option.kind == "allow_always")
                })
                .map(|option| PermissionDecision::Selected(option.id.clone()))
                .or(Some(PermissionDecision::Cancelled)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    Selected(String),
    Cancelled,
}

/// Receives streamed conversation updates and answers permission prompts.
pub trait ConversationObserver: Send + Sync {
    fn on_event(&self, event: ConversationEvent) -> Result<(), String>;
    fn request_permission(&self, request: PermissionRequest) -> Result<PermissionDecision, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversationError {
    AuthenticationRequired(String),
    Other(String),
}

impl std::fmt::Display for ConversationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthenticationRequired(message) | Self::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ConversationError {}

#[cfg(test)]
mod permission_policy_tests {
    use super::{
        PermissionDecision, PermissionOption, PermissionPolicy, PermissionRequest,
    };

    fn option(id: &str, kind: &str) -> PermissionOption {
        PermissionOption {
            id: id.into(),
            name: id.into(),
            kind: kind.into(),
        }
    }

    fn request(options: Vec<PermissionOption>) -> PermissionRequest {
        PermissionRequest {
            title: "Run tool?".into(),
            options,
        }
    }

    #[test]
    fn ask_is_the_default_and_leaves_decision_to_the_ui() {
        assert_eq!(PermissionPolicy::default(), PermissionPolicy::Ask);
        assert_eq!(
            PermissionPolicy::Ask.decision(&request(vec![option("allow", "allow_once")])),
            None
        );
    }

    #[test]
    fn auto_approve_prefers_allow_once_even_when_deny_appears_first() {
        let request = request(vec![
            option("deny", "reject_once"),
            option("allow-always", "allow_always"),
            option("allow-once", "allow_once"),
        ]);
        assert_eq!(
            PermissionPolicy::AutoApprove.decision(&request),
            Some(PermissionDecision::Selected("allow-once".into()))
        );
    }

    #[test]
    fn auto_approve_falls_back_to_allow_always() {
        assert_eq!(
            PermissionPolicy::AutoApprove
                .decision(&request(vec![option("allow-always", "allow_always")])),
            Some(PermissionDecision::Selected("allow-always".into()))
        );
    }

    #[test]
    fn auto_approve_cancels_when_no_grant_option_exists() {
        assert_eq!(
            PermissionPolicy::AutoApprove
                .decision(&request(vec![option("deny", "reject_once")])),
            Some(PermissionDecision::Cancelled)
        );
        assert_eq!(
            PermissionPolicy::AutoApprove.decision(&request(Vec::new())),
            Some(PermissionDecision::Cancelled)
        );
    }
}
