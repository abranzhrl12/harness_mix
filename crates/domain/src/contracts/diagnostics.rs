use super::Request;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ProcessListFilter {
    Active,
    Recent,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ProcessStatusArgs {
    pub pid: Option<u32>,
    #[serde(rename = "runId")]
    pub run_id: Option<String>,
    pub list: Option<ProcessListFilter>,
    pub limit: Option<usize>,
    pub port: Option<u16>,
    pub host: Option<String>,
    #[serde(rename = "healthUrl")]
    pub health_url: Option<String>,
    #[serde(rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsArgs {
    pub files: Option<Vec<String>>,
    #[serde(rename = "maxFiles")]
    pub max_files: Option<usize>,
    #[serde(rename = "buildScope")]
    pub build_scope: Option<String>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DoctorEnvArgs {
    /// Defaults to false: locating tools never starts a subprocess.
    pub versions: Option<bool>,
    pub connection: Option<bool>,
    pub session: Option<String>,
}

pub type DoctorArgs = DoctorEnvArgs;

/// Unified system diagnostics, file syntax checks, log inspection, and process/health probing.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DoctorToolArgs {
    Env(DoctorEnvArgs),
    Syntax(DiagnosticsArgs),
    Logs(LogsArgs),
    Process(ProcessStatusArgs),
    Health(ProcessStatusArgs),
}

impl<'de> serde::Deserialize<'de> for DoctorToolArgs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let action = value
            .as_object_mut()
            .and_then(|obj| obj.remove("action"))
            .and_then(|v| v.as_str().map(String::from));
        match action.as_deref() {
            Some("syntax") => serde_json::from_value(value)
                .map(Self::Syntax)
                .map_err(serde::de::Error::custom),
            Some("logs") => serde_json::from_value(value)
                .map(Self::Logs)
                .map_err(serde::de::Error::custom),
            Some("process") => serde_json::from_value(value)
                .map(Self::Process)
                .map_err(serde::de::Error::custom),
            Some("health") => serde_json::from_value(value)
                .map(Self::Health)
                .map_err(serde::de::Error::custom),
            Some("env") | None => serde_json::from_value(value)
                .map(Self::Env)
                .map_err(serde::de::Error::custom),
            Some(unknown) => Err(serde::de::Error::custom(format!(
                "unknown action: {unknown}"
            ))),
        }
    }
}

impl DoctorToolArgs {
    pub fn into_request(self) -> Request {
        match self {
            Self::Env(args) => Request::Doctor(args),
            Self::Syntax(args) => Request::Diagnostics(args),
            Self::Logs(args) => Request::Logs(args),
            Self::Process(args) | Self::Health(args) => Request::ProcessStatus(args),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct LogsArgs {
    pub limit: Option<usize>,
    pub level: Option<String>,
    pub tool: Option<String>,
    #[serde(rename = "slowOnly")]
    pub slow_only: Option<bool>,
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
    pub after: Option<u64>,
    pub session: Option<String>,
}
